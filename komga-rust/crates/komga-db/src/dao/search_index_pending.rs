//! `ERA_SEARCH_INDEX_PENDING`: entities whose search index document is out of date.

use crate::iden::BookTable as B;
use crate::iden::ContentKey as K;
use crate::iden::SearchIndexPendingTable as T;
use crate::iden::{SeriesTable, SetTable};
use komga_types::{SearchEntity, SeriesId};
use sea_query::{
    Cond, Expr, ExprTrait, IntoCondition, IntoTableRef, Order, Query, SqliteQueryBuilder, TableRef,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::{DbError, DbTransaction};

/// Rows written or removed per statement, below SQLite's bound parameter limit.
const CHUNK: usize = 1000;

pub struct SearchIndexPendingDao;

/// A record that the document of an entity must be brought up to its current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingUpdate {
    /// Changes whenever the entity is marked again.
    pub seq: i64,
    pub id: String,
}

impl SearchIndexPendingDao {
    /// Records that the documents of `ids` are out of date. A record already pending for one of
    /// them gets a new `SEQ`, so that applying the old one does not remove it.
    pub async fn mark(
        transaction: &mut DbTransaction,
        entity: SearchEntity,
        ids: &[impl AsRef<str>],
    ) -> Result<(), DbError> {
        for chunk in ids.chunks(CHUNK) {
            let mut query = Query::insert();
            query
                .replace()
                .into_table(T::Table)
                .columns([T::Entity, T::EntityId]);
            for id in chunk {
                query.values_panic([entity.type_name().into(), id.as_ref().into()]);
            }
            let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    /// Records that the documents carrying the metadata of `series_id` are out of date: its own
    /// and those of its oneshot books.
    pub async fn mark_series_metadata(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<(), DbError> {
        Self::mark(transaction, SearchEntity::Series, &[series_id.as_str()]).await?;
        let condition = Expr::col(B::SeriesId)
            .eq(series_id)
            .and(Expr::col(B::Oneshot).eq(true));
        Self::mark_rows(transaction, SearchEntity::Book, B::Table, condition).await
    }

    /// Records that the documents of the `entity` rows of `table` matching `condition` are out of
    /// date.
    pub(crate) async fn mark_rows(
        transaction: &mut DbTransaction,
        entity: SearchEntity,
        table: impl IntoTableRef,
        condition: impl IntoCondition,
    ) -> Result<(), DbError> {
        let rows = Query::select()
            .expr(Expr::val(entity.type_name()))
            .column(K::Id)
            .from(table)
            .cond_where(condition)
            .take();
        let (sql, values) = Query::insert()
            .replace()
            .into_table(T::Table)
            .columns([T::Entity, T::EntityId])
            .select_from(rows)
            .expect("The select has one value per column")
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Records that every document is out of date, for a rebuild.
    pub async fn mark_all(transaction: &mut DbTransaction) -> Result<(), DbError> {
        for entity in SearchEntity::ALL {
            let table: TableRef = match entity {
                SearchEntity::Book => B::Table.into_table_ref(),
                SearchEntity::Series => SeriesTable::Table.into_table_ref(),
                SearchEntity::Collection => SetTable::Collection.into_table_ref(),
                SearchEntity::ReadList => SetTable::Readlist.into_table_ref(),
            };
            Self::mark_rows(transaction, entity, table, Cond::all()).await?;
        }
        Ok(())
    }

    /// The oldest records of `entity` with a `SEQ` above `after`, at most `limit`.
    pub async fn find(
        transaction: &mut DbTransaction,
        entity: SearchEntity,
        after: i64,
        limit: u64,
    ) -> Result<Vec<PendingUpdate>, DbError> {
        let (sql, values) = Query::select()
            .columns([T::Seq, T::EntityId])
            .from(T::Table)
            .and_where(Expr::col(T::Entity).eq(entity.type_name()))
            .and_where(Expr::col(T::Seq).gt(after))
            .order_by(T::Seq, Order::Asc)
            .limit(limit)
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<(i64, String)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|(seq, id)| PendingUpdate { seq, id })
            .collect())
    }

    /// Removes the applied records; an entity marked again since keeps its newer record.
    pub async fn remove(transaction: &mut DbTransaction, seqs: &[i64]) -> Result<(), DbError> {
        for chunk in seqs.chunks(CHUNK) {
            let (sql, values) = Query::delete()
                .from_table(T::Table)
                .and_where(Expr::col(T::Seq).is_in(chunk.iter().copied()))
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    /// Removes the records of `entity` up to `SEQ` `through`, all applied by a rebuild.
    pub async fn remove_through(
        transaction: &mut DbTransaction,
        entity: SearchEntity,
        through: i64,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(T::Table)
            .and_where(Expr::col(T::Entity).eq(entity.type_name()))
            .and_where(Expr::col(T::Seq).lte(through))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use komga_types::SearchEntity;

    use super::{PendingUpdate, SearchIndexPendingDao};
    use crate::Database;

    #[tokio::test]
    async fn an_entity_marked_again_keeps_its_record_when_the_older_one_is_applied() {
        let directory = komga_test_support::tempdir().expect("Database directory");
        let db = Database::connect(&directory.path().join("database.sqlite"))
            .await
            .expect("Open database");
        db.migrate().await.expect("Migrate database");
        db.migrate()
            .await
            .expect("The table is created only when missing");
        let mut tx = db.begin().await.expect("Begin");
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::Collection, &["r"])
            .await
            .expect("Mark another entity");
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::ReadList, &["r"])
            .await
            .expect("Mark");
        let applied = SearchIndexPendingDao::find(&mut tx, SearchEntity::ReadList, 0, 10)
            .await
            .expect("Find");
        assert_eq!(applied.len(), 1, "entities have their own records");

        // The latest record is replaced while being applied: its SEQ is not reused.
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::ReadList, &["r"])
            .await
            .expect("Mark again");
        SearchIndexPendingDao::remove(&mut tx, &[applied[0].seq])
            .await
            .expect("Remove");
        let pending = SearchIndexPendingDao::find(&mut tx, SearchEntity::ReadList, 0, 10)
            .await
            .expect("Find again");
        assert!(
            matches!(&pending[..], [PendingUpdate { seq, id }] if *seq > applied[0].seq && id == "r"),
            "{pending:?}"
        );
    }

    #[tokio::test]
    async fn a_rebuild_removes_only_the_records_it_applied() {
        let directory = komga_test_support::tempdir().expect("Database directory");
        let db = Database::connect(&directory.path().join("database.sqlite"))
            .await
            .expect("Open database");
        db.migrate().await.expect("Migrate database");
        let mut tx = db.begin().await.expect("Begin");
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::ReadList, &["a", "b", "c"])
            .await
            .expect("Mark");
        let first = SearchIndexPendingDao::find(&mut tx, SearchEntity::ReadList, 0, 2)
            .await
            .expect("Find");
        let rest = SearchIndexPendingDao::find(&mut tx, SearchEntity::ReadList, first[1].seq, 2)
            .await
            .expect("Find after");
        assert_eq!(
            [&first[..], &rest[..]]
                .concat()
                .iter()
                .map(|update| update.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        // Marked again after the rebuild applied it.
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::ReadList, &["a"])
            .await
            .expect("Mark again");
        SearchIndexPendingDao::remove_through(&mut tx, SearchEntity::ReadList, rest[0].seq)
            .await
            .expect("Remove");
        let pending = SearchIndexPendingDao::find(&mut tx, SearchEntity::ReadList, 0, 10)
            .await
            .expect("Find again");
        assert_eq!(
            pending
                .iter()
                .map(|update| update.id.as_str())
                .collect::<Vec<_>>(),
            ["a"]
        );
    }
}
