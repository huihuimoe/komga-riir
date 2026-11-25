//! Read list snapshot creation, diff queries and sync marking for `SyncPointDao`.

use crate::iden::BookTable as B;
use crate::iden::ReadProgressSeriesTable as RS;
use crate::iden::SyncPointReadListBookTable as SPRLB;
use crate::iden::SyncPointReadListRemovedSyncedTable as SPRLR;
use crate::iden::SyncPointReadListTable as SPRL;
use komga_types::{
    BookId, ContentScope, ON_DECK_READ_LIST_ID, SyncPoint, SyncPointId, SyncPointReadList,
    SyncPointReadLists,
};
use sea_query::{
    Cond, Condition, Expr, ExprTrait, Func, JoinType, Order, Query, SelectStatement,
    SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{Snapshot, SyncPointDao, execute, ids, page};
use crate::connection::DbTransaction;
use crate::dao::book::dto::select_on_deck;
use crate::error::DbError;
use crate::support::local_date_time::SqlDateTime;

const READ_LIST_COLUMNS: [SPRL; 5] = [
    SPRL::ReadListId,
    SPRL::ReadListName,
    SPRL::ReadListCreatedDate,
    SPRL::ReadListLastModifiedDate,
    SPRL::Synced,
];

type ReadListRow = (String, String, SqlDateTime, SqlDateTime, bool);

impl SyncPointDao {
    /// `addOnDeck`: the user's On Deck books in `scope` as a read list of the sync point, last
    /// modified when they last read one of those series; none when there are no such books.
    pub async fn add_on_deck(
        transaction: &mut DbTransaction,
        sync_point: &SyncPoint,
        scope: &ContentScope,
    ) -> Result<(), DbError> {
        let on_deck = select_on_deck(&sync_point.user_id, scope, None);
        let created = SqlDateTime(sync_point.created_date);
        let mut books = on_deck.clone();
        books
            .clear_selects()
            .expr(Expr::val(sync_point.id.clone()))
            .expr(Expr::val(ON_DECK_READ_LIST_ID))
            .column((B::Table, B::Id));
        let insert_books = Query::insert()
            .into_table(SPRLB::Table)
            .columns([SPRLB::SyncPointId, SPRLB::ReadListId, SPRLB::BookId])
            .select_from(books)
            .expect("The select has one value per column")
            .build_sqlx(SqliteQueryBuilder);
        if execute(transaction, insert_books).await? == 0 {
            return Ok(());
        }
        let mut read_list = on_deck;
        read_list
            .clear_selects()
            .expr(Expr::val(sync_point.id.clone()))
            .expr(Expr::val(ON_DECK_READ_LIST_ID))
            .expr(Expr::val("On Deck"))
            .expr(Expr::val(created))
            .expr(Func::coalesce([
                Expr::from(Func::max(Expr::col((RS::Table, RS::MostRecentReadDate)))),
                Expr::val(created),
            ]));
        let insert_read_list = Query::insert()
            .into_table(SPRL::Table)
            .columns([
                SPRL::SyncPointId,
                SPRL::ReadListId,
                SPRL::ReadListName,
                SPRL::ReadListCreatedDate,
                SPRL::ReadListLastModifiedDate,
            ])
            .select_from(read_list)
            .expect("The select has one value per column")
            .build_sqlx(SqliteQueryBuilder);
        execute(transaction, insert_read_list).await?;
        Ok(())
    }

    /// `takeReadLists*`: as [`Self::take_books`], for read lists.
    pub async fn take_read_lists(
        transaction: &mut DbTransaction,
        to: &SyncPointId,
        read_lists: SyncPointReadLists<'_>,
        size: u32,
    ) -> Result<(Vec<SyncPointReadList>, u64), DbError> {
        let query = read_list_query(to, read_lists);
        let (rows, total) =
            page::<ReadListRow>(transaction, query, (Snapshot::To, SPRL::ReadListId), size).await?;
        let items: Vec<SyncPointReadList> = rows.into_iter().map(read_list).collect();
        if !items.is_empty() {
            mark_read_lists_synced(
                transaction,
                to,
                &items,
                matches!(read_lists, SyncPointReadLists::RemovedSince(_)),
            )
            .await?;
        }
        Ok((items, total))
    }

    /// `findBookIdsByReadListIds`: the books of `read_list_ids` in the sync point, as
    /// `(read list ID, book ID)`.
    pub async fn find_read_list_books(
        transaction: &mut DbTransaction,
        id: &SyncPointId,
        read_list_ids: &[&str],
    ) -> Result<Vec<(String, BookId)>, DbError> {
        let (sql, values) = Query::select()
            .columns([SPRLB::ReadListId, SPRLB::BookId])
            .from(SPRLB::Table)
            .and_where(Expr::col(SPRLB::SyncPointId).eq(id.clone()))
            .and_where(Expr::col(SPRLB::ReadListId).is_in(read_list_ids.iter().copied()))
            .order_by(SPRLB::ReadListId, Order::Asc)
            .order_by(SPRLB::BookId, Order::Asc)
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }
}

fn read_list_query(to: &SyncPointId, read_lists: SyncPointReadLists<'_>) -> SelectStatement {
    let (x, f) = (Snapshot::To, Snapshot::From);
    let mut query = Query::select();
    query
        .columns(READ_LIST_COLUMNS.map(|column| (x, column)))
        .from_as(SPRL::Table, x);
    match read_lists {
        SyncPointReadLists::All => {
            unsynced_read_lists(&mut query, to);
        }
        SyncPointReadLists::AddedSince(from) => {
            unsynced_read_lists(&mut query, to).and_where(
                Expr::col((x, SPRL::ReadListId)).not_in_subquery(ids(
                    SPRL::Table,
                    SPRL::SyncPointId,
                    SPRL::ReadListId,
                    from,
                )),
            );
        }
        SyncPointReadLists::ChangedSince(from) => {
            query.join_as(
                JoinType::InnerJoin,
                SPRL::Table,
                f,
                Expr::col((f, SPRL::SyncPointId))
                    .eq(from.clone())
                    .and(Expr::col((f, SPRL::ReadListId)).equals((x, SPRL::ReadListId))),
            );
            unsynced_read_lists(&mut query, to).cond_where(read_list_changed());
        }
        SyncPointReadLists::RemovedSince(from) => {
            query
                .and_where(Expr::col((x, SPRL::SyncPointId)).eq(from.clone()))
                .and_where(Expr::col((x, SPRL::ReadListId)).not_in_subquery(ids(
                    SPRL::Table,
                    SPRL::SyncPointId,
                    SPRL::ReadListId,
                    to,
                )))
                .and_where(Expr::col((x, SPRL::ReadListId)).not_in_subquery(ids(
                    SPRLR::Table,
                    SPRLR::SyncPointId,
                    SPRLR::ReadListId,
                    to,
                )));
        }
    }
    query
}

/// Read lists of `to` not handed out yet.
fn unsynced_read_lists<'q>(
    query: &'q mut SelectStatement,
    to: &SyncPointId,
) -> &'q mut SelectStatement {
    query
        .and_where(Expr::col((Snapshot::To, SPRL::SyncPointId)).eq(to.clone()))
        .and_where(Expr::col((Snapshot::To, SPRL::Synced)).eq(false))
}

/// Whether the read list's name or modification date changed between the snapshots.
fn read_list_changed() -> Condition {
    let (x, f) = (Snapshot::To, Snapshot::From);
    Cond::any()
        .add(
            Expr::col((x, SPRL::ReadListLastModifiedDate))
                .is_not(Expr::col((f, SPRL::ReadListLastModifiedDate))),
        )
        .add(Expr::col((x, SPRL::ReadListName)).is_not(Expr::col((f, SPRL::ReadListName))))
}

async fn mark_read_lists_synced(
    transaction: &mut DbTransaction,
    to: &SyncPointId,
    items: &[SyncPointReadList],
    removed: bool,
) -> Result<(), DbError> {
    let ids = items.iter().map(|read_list| read_list.read_list_id.clone());
    let mark = if removed {
        let mut insert = Query::insert();
        insert
            .into_table(SPRLR::Table)
            .columns([SPRLR::SyncPointId, SPRLR::ReadListId]);
        for id in ids {
            insert.values_panic([to.clone().into(), id.into()]);
        }
        insert.build_sqlx(SqliteQueryBuilder)
    } else {
        Query::update()
            .table(SPRL::Table)
            .value(SPRL::Synced, true)
            .and_where(Expr::col(SPRL::SyncPointId).eq(to.clone()))
            .and_where(Expr::col(SPRL::ReadListId).is_in(ids))
            .build_sqlx(SqliteQueryBuilder)
    };
    execute(transaction, mark).await?;
    Ok(())
}

fn read_list(row: ReadListRow) -> SyncPointReadList {
    let (read_list_id, name, created, last_modified, synced) = row;
    SyncPointReadList {
        read_list_id,
        name,
        created_date: created.0,
        last_modified_date: last_modified.0,
        synced,
    }
}
