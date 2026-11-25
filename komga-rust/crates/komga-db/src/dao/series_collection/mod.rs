//! Kotlin `SeriesCollectionDao`.

use crate::iden::{SetKey as K, SetTable as T};
use komga_types::{CollectionId, SeriesId};
use sea_query::{Expr, ExprTrait, Func, Order, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::support::local_date_time;
use crate::{DbError, DbTransaction};

mod visible;

pub struct SeriesCollectionDao;

/// A collection with every series, ordered by number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSeries {
    pub id: CollectionId,
    /// `(NUMBER, SERIES_ID)` pairs.
    pub series: Vec<(i32, SeriesId)>,
}

impl SeriesCollectionDao {
    /// `findByNameOrNull`: the collection whose name equals `name` under SQLite's ASCII-only
    /// `lower`, as jOOQ's `equalIgnoreCase` compares.
    pub async fn find_by_name(
        transaction: &mut DbTransaction,
        name: &str,
    ) -> Result<Option<CollectionSeries>, DbError> {
        let (sql, values) = Query::select()
            .column(K::Id)
            .from(T::Collection)
            .and_where(Expr::expr(Func::lower(Expr::col(K::Name))).eq(Func::lower(name)))
            .order_by(K::Id, Order::Asc)
            .limit(1)
            .build_sqlx(SqliteQueryBuilder);
        let found: Option<CollectionId> =
            sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(transaction.connection())
                .await?;
        let Some(id) = found else {
            return Ok(None);
        };
        let (sql, values) = Query::select()
            .columns([K::Number, K::SeriesId])
            .from(T::CollectionSeries)
            .and_where(Expr::col(K::CollectionId).eq(&id))
            .order_by(K::Number, Order::Asc)
            .build_sqlx(SqliteQueryBuilder);
        let series: Vec<(i32, SeriesId)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok(Some(CollectionSeries { id, series }))
    }

    /// `insert`: a new, not manually ordered collection holding `series`.
    pub async fn insert_with_series(
        transaction: &mut DbTransaction,
        id: &CollectionId,
        name: &str,
        series: &SeriesId,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(T::Collection)
            .columns([K::Id, K::Name, K::SeriesCount])
            .values_panic([id.into(), name.into(), 1.into()])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        insert_series(transaction, id, 0, series).await
    }

    /// `update` with one more series: adds `series` at `number`, then recounts the series and
    /// stamps `LAST_MODIFIED_DATE`.
    pub async fn add_series(
        transaction: &mut DbTransaction,
        id: &CollectionId,
        number: i32,
        series: &SeriesId,
    ) -> Result<(), DbError> {
        insert_series(transaction, id, number, series).await?;
        let count = Query::select()
            .expr(Expr::col(K::SeriesId).count())
            .from(T::CollectionSeries)
            .and_where(Expr::col(K::CollectionId).eq(id))
            .take();
        let (sql, values) = Query::update()
            .table(T::Collection)
            .value(K::SeriesCount, count)
            .value(K::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(K::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `findAllEmpty` and `delete`: removes collections without series, with their thumbnails,
    /// and returns their IDs.
    pub async fn delete_empty(
        transaction: &mut DbTransaction,
    ) -> Result<Vec<CollectionId>, DbError> {
        let members = Query::select()
            .column(K::CollectionId)
            .from(T::CollectionSeries)
            .take();
        let (sql, values) = Query::select()
            .column(K::Id)
            .from(T::Collection)
            .and_where(Expr::col(K::Id).not_in_subquery(members))
            .build_sqlx(SqliteQueryBuilder);
        let ids: Vec<CollectionId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        delete_all(transaction, &ids).await?;
        Ok(ids)
    }

    /// `existsByName` under the same case folding as [`Self::find_by_name`], ignoring the
    /// collection `except` so that it may keep or recase its own name.
    pub async fn exists_by_name(
        transaction: &mut DbTransaction,
        name: &str,
        except: Option<&CollectionId>,
    ) -> Result<bool, DbError> {
        let mut query = Query::select();
        query
            .expr(Expr::val(1))
            .from(T::Collection)
            .and_where(Expr::expr(Func::lower(Expr::col(K::Name))).eq(Func::lower(name)));
        if let Some(except) = except {
            query.and_where(Expr::col(K::Id).ne(except));
        }
        let (sql, values) = query.limit(1).build_sqlx(SqliteQueryBuilder);
        let found: Option<i32> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(found.is_some())
    }

    /// `insert`: the series are numbered from 0 in the given order.
    pub async fn insert(
        transaction: &mut DbTransaction,
        id: &CollectionId,
        name: &str,
        ordered: bool,
        series: &[SeriesId],
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(T::Collection)
            .columns([K::Id, K::Name, K::Ordered, K::SeriesCount])
            .values_panic([
                id.into(),
                name.into(),
                ordered.into(),
                series_count(series).into(),
            ])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        insert_all_series(transaction, id, series).await
    }

    /// `update`, stamping `LAST_MODIFIED_DATE`. With `series`, they replace every member,
    /// numbered from 0; without, the members stay as they are.
    pub async fn update(
        transaction: &mut DbTransaction,
        id: &CollectionId,
        name: &str,
        ordered: bool,
        series: Option<&[SeriesId]>,
    ) -> Result<(), DbError> {
        let mut query = Query::update();
        query
            .table(T::Collection)
            .value(K::Name, name)
            .value(K::Ordered, ordered)
            .value(K::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(K::Id).eq(id));
        if let Some(series) = series {
            query.value(K::SeriesCount, series_count(series));
        }
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        let Some(series) = series else {
            return Ok(());
        };
        let (sql, values) = Query::delete()
            .from_table(T::CollectionSeries)
            .and_where(Expr::col(K::CollectionId).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        insert_all_series(transaction, id, series).await
    }

    /// `(ID, NAME)` of each of `ids` that exists, for its search document.
    pub async fn find_names(
        transaction: &mut DbTransaction,
        ids: &[&str],
    ) -> Result<Vec<(String, String)>, DbError> {
        let (sql, values) = Query::select()
            .columns([K::Id, K::Name])
            .from(T::Collection)
            .and_where(Expr::col(K::Id).is_in(ids.iter().copied()))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// `ThumbnailSeriesCollectionDao.deleteByCollectionId` and `delete`.
    pub async fn delete(transaction: &mut DbTransaction, id: &CollectionId) -> Result<(), DbError> {
        delete_all(transaction, std::slice::from_ref(id)).await
    }
}

/// Deletes collections with their members and thumbnails.
async fn delete_all(transaction: &mut DbTransaction, ids: &[CollectionId]) -> Result<(), DbError> {
    for (table, column) in [
        (T::ThumbnailCollection, K::CollectionId),
        (T::CollectionSeries, K::CollectionId),
        (T::Collection, K::Id),
    ] {
        let (sql, values) = Query::delete()
            .from_table(table)
            .and_where(Expr::col(column).is_in(ids))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
    }
    Ok(())
}

async fn insert_all_series(
    transaction: &mut DbTransaction,
    id: &CollectionId,
    series: &[SeriesId],
) -> Result<(), DbError> {
    for (number, series) in (0..).zip(series) {
        insert_series(transaction, id, number, series).await?;
    }
    Ok(())
}

fn series_count(series: &[SeriesId]) -> i32 {
    i32::try_from(series.len()).expect("A collection holds fewer than 2^31 series")
}

async fn insert_series(
    transaction: &mut DbTransaction,
    id: &CollectionId,
    number: i32,
    series: &SeriesId,
) -> Result<(), DbError> {
    let (sql, values) = Query::insert()
        .into_table(T::CollectionSeries)
        .columns([K::CollectionId, K::SeriesId, K::Number])
        .values_panic([id.into(), series.into(), number.into()])
        .build_sqlx(SqliteQueryBuilder);
    sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .execute(transaction.connection())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SeriesCollectionDao;
    use crate::Database;

    #[tokio::test]
    async fn deletes_only_collections_without_series_and_their_thumbnails() {
        let directory = komga_test_support::tempdir().expect("Database directory");
        let db = Database::connect(&directory.path().join("database.sqlite"))
            .await
            .expect("Open database");
        db.migrate().await.expect("Migrate database");
        let mut tx = db.begin().await.expect("Begin");
        for statement in [
            "INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('l', 'L', 'file:/l')",
            "INSERT INTO SERIES (ID, FILE_LAST_MODIFIED, NAME, URL, LIBRARY_ID) \
             VALUES ('s', CURRENT_TIMESTAMP, 'S', 'file:/l/s', 'l')",
            "INSERT INTO COLLECTION (ID, NAME, SERIES_COUNT) VALUES ('kept', 'K', 1), ('empty', 'E', 0)",
            "INSERT INTO COLLECTION_SERIES (COLLECTION_ID, SERIES_ID, NUMBER) VALUES ('kept', 's', 0)",
            "INSERT INTO THUMBNAIL_COLLECTION (ID, THUMBNAIL, TYPE, COLLECTION_ID) \
             VALUES ('t1', x'00', 'USER_UPLOADED', 'kept'), ('t2', x'00', 'USER_UPLOADED', 'empty')",
        ] {
            sqlx::query(statement)
                .execute(tx.connection())
                .await
                .expect("Fixture");
        }
        assert_eq!(
            SeriesCollectionDao::delete_empty(&mut tx)
                .await
                .expect("Delete"),
            [komga_types::CollectionId::from("empty".to_owned())]
        );
        let remaining: Vec<(String, String)> = sqlx::query_as(
            "SELECT c.ID, t.ID FROM COLLECTION c JOIN THUMBNAIL_COLLECTION t ON t.COLLECTION_ID = c.ID",
        )
        .fetch_all(tx.connection())
        .await
        .expect("Remaining");
        assert_eq!(remaining, [("kept".to_owned(), "t1".to_owned())]);
        let thumbnails: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM THUMBNAIL_COLLECTION")
            .fetch_one(tx.connection())
            .await
            .expect("Thumbnails");
        assert_eq!(thumbnails, 1, "the empty collection's thumbnail is deleted");
    }
}
