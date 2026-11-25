//! Kotlin `SyncPointDao`: book and read list snapshots, compared between two sync points.
//!
//! Unlike Kotlin, comparisons are null-safe (`IS NOT`), a hash is compared only once the earlier
//! snapshot has one, and pages are ordered by ID.

mod books;
mod read_lists;

use crate::iden::BookMetadataTable as D;
use crate::iden::BookTable as B;
use crate::iden::ReadProgressTable as R;
use crate::iden::SyncPointBookRemovedSyncedTable as SPBR;
use crate::iden::SyncPointBookTable as SPB;
use crate::iden::SyncPointReadListBookTable as SPRLB;
use crate::iden::SyncPointReadListRemovedSyncedTable as SPRLR;
use crate::iden::SyncPointReadListTable as SPRL;
use crate::iden::SyncPointTable as SP;
use crate::iden::ThumbnailBookTable as BT;
use komga_types::{ApiKeyId, BookCondition, ContentScope, SyncPoint, SyncPointId, UserId};
use sea_query::{
    Cond, Condition, DynIden, Expr, ExprTrait, Iden, IntoColumnRef, IntoIden, IntoTableRef, Order,
    Query, SelectStatement, SqliteQueryBuilder,
};
use sea_query_sqlx::{SqlxBinder, SqlxValues};
use sqlx::AssertSqlSafe;

use crate::connection::{Database, DbTransaction};
use crate::dao::book::dto::select_matching;
use crate::error::DbError;
use crate::support::local_date_time::{self, SqlDateTime};
use crate::support::rows::count;

/// The snapshots compared: `x` of the sync point taken from, `f` of the earlier one.
#[derive(Clone, Copy, Iden)]
enum Snapshot {
    #[iden = "x"]
    To,
    #[iden = "f"]
    From,
}

pub struct SyncPointDao;

impl SyncPointDao {
    /// `create`: a sync point of `user_id` over the books in `scope` matching `condition`, as
    /// they and the user's progress on them are now.
    pub async fn create(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        api_key_id: Option<&ApiKeyId>,
        scope: &ContentScope,
        condition: &BookCondition,
    ) -> Result<SyncPoint, DbError> {
        let sync_point = SyncPoint {
            id: SyncPointId::generate(),
            user_id: user_id.clone(),
            api_key_id: api_key_id.cloned(),
            created_date: local_date_time::now_utc().0,
        };
        let insert = Query::insert()
            .into_table(SP::Table)
            .columns([SP::Id, SP::UserId, SP::ApiKeyId, SP::CreatedDate])
            .values_panic([
                sync_point.id.clone().into(),
                user_id.clone().into(),
                api_key_id.map(ApiKeyId::as_str).into(),
                SqlDateTime(sync_point.created_date).into(),
            ])
            .build_sqlx(SqliteQueryBuilder);
        execute(transaction, insert).await?;
        insert_book_snapshot(transaction, &sync_point.id, user_id, scope, condition).await?;
        Ok(sync_point)
    }

    /// `findByIdOrNull`.
    pub async fn find_by_id(
        database: &Database,
        id: &SyncPointId,
    ) -> Result<Option<SyncPoint>, DbError> {
        let (sql, values) = Query::select()
            .columns([SP::Id, SP::UserId, SP::ApiKeyId, SP::CreatedDate])
            .from(SP::Table)
            .and_where(Expr::col(SP::Id).eq(id.clone()))
            .build_sqlx(SqliteQueryBuilder);
        let row: Option<(SyncPointId, UserId, Option<ApiKeyId>, SqlDateTime)> =
            sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(database.read_pool())
                .await?;
        Ok(row.map(|(id, user_id, api_key_id, created)| SyncPoint {
            id,
            user_id,
            api_key_id,
            created_date: created.0,
        }))
    }

    /// `deleteOne`.
    pub async fn delete(transaction: &mut DbTransaction, id: &SyncPointId) -> Result<(), DbError> {
        delete_where(transaction, Expr::col(SP::Id).eq(id.clone()).into()).await
    }

    /// `deleteByUserId`, or `deleteByUserIdAndApiKeyIds` when `api_key_ids` is given.
    pub async fn delete_by_user(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        api_key_ids: Option<&[ApiKeyId]>,
    ) -> Result<(), DbError> {
        let mut condition = Cond::all().add(Expr::col(SP::UserId).eq(user_id.clone()));
        if let Some(api_key_ids) = api_key_ids {
            condition = condition.add(Expr::col(SP::ApiKeyId).is_in(api_key_ids.iter().cloned()));
        }
        delete_where(transaction, condition).await
    }
}

async fn insert_book_snapshot(
    transaction: &mut DbTransaction,
    sync_point_id: &SyncPointId,
    user_id: &UserId,
    scope: &ContentScope,
    condition: &BookCondition,
) -> Result<(), DbError> {
    let mut books = select_matching(user_id, scope, condition);
    books
        .clear_selects()
        .expr(Expr::val(sync_point_id.clone()))
        .columns(
            [
                B::Id,
                B::CreatedDate,
                B::LastModifiedDate,
                B::FileLastModified,
                B::FileSize,
                B::FileHash,
            ]
            .map(|column| (B::Table, column)),
        )
        .column((D::Table, D::LastModifiedDate))
        .column((R::Table, R::LastModifiedDate))
        .column((BT::Table, BT::Id))
        .left_join(
            BT::Table,
            Expr::col((BT::Table, BT::BookId))
                .equals((B::Table, B::Id))
                .and(Expr::col((BT::Table, BT::Selected)).eq(true)),
        );
    let snapshot = Query::insert()
        .into_table(SPB::Table)
        .columns([
            SPB::SyncPointId,
            SPB::BookId,
            SPB::BookCreatedDate,
            SPB::BookLastModifiedDate,
            SPB::BookFileLastModified,
            SPB::BookFileSize,
            SPB::BookFileHash,
            SPB::BookMetadataLastModifiedDate,
            SPB::BookReadProgressLastModifiedDate,
            SPB::BookThumbnailId,
        ])
        .select_from(books)
        .expect("The select has one value per column")
        .build_sqlx(SqliteQueryBuilder);
    execute(transaction, snapshot).await?;
    Ok(())
}

/// Deletes the sync points matching `condition`, with their snapshots.
async fn delete_where(
    transaction: &mut DbTransaction,
    condition: Condition,
) -> Result<(), DbError> {
    let sync_points = Query::select()
        .column(SP::Id)
        .from(SP::Table)
        .cond_where(condition.clone())
        .take();
    let children: [(DynIden, DynIden); 5] = [
        (SPRLR::Table.into_iden(), SPRLR::SyncPointId.into_iden()),
        (SPRLB::Table.into_iden(), SPRLB::SyncPointId.into_iden()),
        (SPRL::Table.into_iden(), SPRL::SyncPointId.into_iden()),
        (SPBR::Table.into_iden(), SPBR::SyncPointId.into_iden()),
        (SPB::Table.into_iden(), SPB::SyncPointId.into_iden()),
    ];
    for (table, column) in children {
        let delete = Query::delete()
            .from_table(table)
            .and_where(Expr::col(column).in_subquery(sync_points.clone()))
            .build_sqlx(SqliteQueryBuilder);
        execute(transaction, delete).await?;
    }
    let delete = Query::delete()
        .from_table(SP::Table)
        .cond_where(condition)
        .build_sqlx(SqliteQueryBuilder);
    execute(transaction, delete).await?;
    Ok(())
}

/// The `id`s of `sync_point` in `table`.
fn ids(
    table: impl IntoTableRef,
    sync_point_column: impl IntoColumnRef,
    id: impl IntoColumnRef,
    sync_point: &SyncPointId,
) -> SelectStatement {
    Query::select()
        .column(id)
        .from(table)
        .and_where(Expr::col(sync_point_column).eq(sync_point.clone()))
        .take()
}

/// Up to `size` rows of `query` by `order`, and how many rows there are.
async fn page<T>(
    transaction: &mut DbTransaction,
    query: SelectStatement,
    order: impl IntoColumnRef,
    size: u32,
) -> Result<(Vec<T>, u64), DbError>
where
    T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
{
    let (sql, values) = query
        .clone()
        .order_by(order, Order::Asc)
        .limit(u64::from(size))
        .build_sqlx(SqliteQueryBuilder);
    let rows = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(transaction.connection())
        .await?;
    Ok((rows, count(transaction, query).await?))
}

async fn execute(
    transaction: &mut DbTransaction,
    (sql, values): (String, SqlxValues),
) -> Result<u64, DbError> {
    let result = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .execute(transaction.connection())
        .await?;
    Ok(result.rows_affected())
}
