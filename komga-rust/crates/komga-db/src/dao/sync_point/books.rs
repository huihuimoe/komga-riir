//! Book snapshot diff queries and sync marking for `SyncPointDao`.

use crate::iden::SyncPointBookRemovedSyncedTable as SPBR;
use crate::iden::SyncPointBookTable as SPB;
use komga_types::{BookId, SyncPointBook, SyncPointBooks, SyncPointId, ThumbnailId};
use sea_query::{
    Cond, Condition, Expr, ExprTrait, IntoIden, JoinType, Query, SelectStatement,
    SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;

use super::{Snapshot, SyncPointDao, execute, ids, page};
use crate::connection::DbTransaction;
use crate::error::DbError;
use crate::support::local_date_time::SqlDateTime;

const BOOK_COLUMNS: [SPB; 9] = [
    SPB::BookId,
    SPB::BookCreatedDate,
    SPB::BookLastModifiedDate,
    SPB::BookFileLastModified,
    SPB::BookFileSize,
    SPB::BookFileHash,
    SPB::BookMetadataLastModifiedDate,
    SPB::BookThumbnailId,
    SPB::Synced,
];

type BookRow = (
    BookId,
    SqlDateTime,
    SqlDateTime,
    SqlDateTime,
    i64,
    String,
    SqlDateTime,
    Option<ThumbnailId>,
    bool,
);

impl SyncPointDao {
    /// `takeBooks*`: up to `size` books of `to` not yet handed out, by ID, which are then marked
    /// handed out, with how many there were before.
    pub async fn take_books(
        transaction: &mut DbTransaction,
        to: &SyncPointId,
        books: SyncPointBooks<'_>,
        size: u32,
    ) -> Result<(Vec<SyncPointBook>, u64), DbError> {
        let query = book_query(to, books);
        let (rows, total) =
            page::<BookRow>(transaction, query, (Snapshot::To, SPB::BookId), size).await?;
        let items: Vec<SyncPointBook> = rows.into_iter().map(book).collect();
        if !items.is_empty() {
            mark_books_synced(
                transaction,
                to,
                &items,
                matches!(books, SyncPointBooks::RemovedSince(_)),
            )
            .await?;
        }
        Ok((items, total))
    }
}

fn book_query(to: &SyncPointId, books: SyncPointBooks<'_>) -> SelectStatement {
    let x = Snapshot::To;
    let mut query = Query::select();
    query
        .columns(BOOK_COLUMNS.map(|column| (x, column)))
        .from_as(SPB::Table, x);
    match books {
        SyncPointBooks::All => {
            unsynced_books(&mut query, to);
        }
        SyncPointBooks::AddedSince(from) => {
            unsynced_books(&mut query, to).and_where(
                Expr::col((x, SPB::BookId)).not_in_subquery(ids(
                    SPB::Table,
                    SPB::SyncPointId,
                    SPB::BookId,
                    from,
                )),
            );
        }
        SyncPointBooks::ChangedSince(from) => {
            compared_books(&mut query, to, from).cond_where(book_changed());
        }
        SyncPointBooks::ReadProgressChangedSince(from) => {
            compared_books(&mut query, to, from)
                .cond_where(book_changed().not())
                .and_where(differs(SPB::BookReadProgressLastModifiedDate));
        }
        SyncPointBooks::RemovedSince(from) => {
            query
                .and_where(Expr::col((x, SPB::SyncPointId)).eq(from.clone()))
                .and_where(Expr::col((x, SPB::BookId)).not_in_subquery(ids(
                    SPB::Table,
                    SPB::SyncPointId,
                    SPB::BookId,
                    to,
                )))
                .and_where(Expr::col((x, SPB::BookId)).not_in_subquery(ids(
                    SPBR::Table,
                    SPBR::SyncPointId,
                    SPBR::BookId,
                    to,
                )));
        }
    }
    query
}

async fn mark_books_synced(
    transaction: &mut DbTransaction,
    to: &SyncPointId,
    items: &[SyncPointBook],
    removed: bool,
) -> Result<(), DbError> {
    let ids = items.iter().map(|book| book.book_id.clone());
    let mark = if removed {
        let mut insert = Query::insert();
        insert
            .into_table(SPBR::Table)
            .columns([SPBR::SyncPointId, SPBR::BookId]);
        for id in ids {
            insert.values_panic([to.clone().into(), id.into()]);
        }
        insert.build_sqlx(SqliteQueryBuilder)
    } else {
        Query::update()
            .table(SPB::Table)
            .value(SPB::Synced, true)
            .and_where(Expr::col(SPB::SyncPointId).eq(to.clone()))
            .and_where(Expr::col(SPB::BookId).is_in(ids))
            .build_sqlx(SqliteQueryBuilder)
    };
    execute(transaction, mark).await?;
    Ok(())
}

/// Books of `to` not handed out yet.
fn unsynced_books<'q>(query: &'q mut SelectStatement, to: &SyncPointId) -> &'q mut SelectStatement {
    query
        .and_where(Expr::col((Snapshot::To, SPB::SyncPointId)).eq(to.clone()))
        .and_where(Expr::col((Snapshot::To, SPB::Synced)).eq(false))
}

/// Books of `to` not handed out yet, with their snapshot in `from`.
fn compared_books<'q>(
    query: &'q mut SelectStatement,
    to: &SyncPointId,
    from: &SyncPointId,
) -> &'q mut SelectStatement {
    let (x, f) = (Snapshot::To, Snapshot::From);
    query.join_as(
        JoinType::InnerJoin,
        SPB::Table,
        f,
        Expr::col((f, SPB::SyncPointId))
            .eq(from.clone())
            .and(Expr::col((f, SPB::BookId)).equals((x, SPB::BookId))),
    );
    unsynced_books(query, to)
}

/// Whether the book or its thumbnail changed between the snapshots.
fn book_changed() -> Condition {
    Cond::any()
        .add(differs(SPB::BookFileLastModified))
        .add(differs(SPB::BookFileSize))
        .add(
            Expr::col((Snapshot::From, SPB::BookFileHash))
                .ne("")
                .and(differs(SPB::BookFileHash)),
        )
        .add(differs(SPB::BookMetadataLastModifiedDate))
        .add(differs(SPB::BookThumbnailId))
}

fn differs(column: SPB) -> Expr {
    let column = column.into_iden();
    Expr::col((Snapshot::To, column.clone())).is_not(Expr::col((Snapshot::From, column)))
}

fn book(row: BookRow) -> SyncPointBook {
    let (
        book_id,
        created,
        last_modified,
        file_last_modified,
        file_size,
        file_hash,
        metadata,
        thumbnail_id,
        synced,
    ) = row;
    SyncPointBook {
        book_id,
        created_date: created.0,
        last_modified_date: last_modified.0,
        file_last_modified: file_last_modified.0,
        file_size,
        file_hash,
        metadata_last_modified_date: metadata.0,
        thumbnail_id,
        synced,
    }
}
