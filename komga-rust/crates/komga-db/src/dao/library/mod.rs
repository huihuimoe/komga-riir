//! Library persistence, database-only content cleanup, and restoration of soft-deleted content.

mod deletion;
mod restoration;
mod row;

pub use deletion::DeletedContent;
pub use restoration::RestoreDao;

use std::collections::{BTreeMap, BTreeSet};

use crate::iden::{LibraryExclusionsTable as E, LibraryTable};
use komga_types::{ContentScope, Library, LibraryId};
use sea_query::{Asterisk, Cond, Condition, Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::connection::DbTransaction;
use crate::error::DbError;
use row::LibraryRow;

/// DAO for library records.
pub struct LibraryDao;

impl LibraryDao {
    pub async fn find_all(
        transaction: &mut DbTransaction,
        scope: Option<&ContentScope>,
    ) -> Result<Vec<Library>, DbError> {
        let mut condition = Cond::all();
        if let Some(ids) = scope.and_then(ContentScope::library_ids) {
            condition =
                condition.add(Expr::col((LibraryTable::Table, LibraryTable::Id)).is_in(ids));
        }
        Self::find(transaction, condition).await
    }

    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        id: &LibraryId,
    ) -> Result<Option<Library>, DbError> {
        let condition = Cond::all().add(Expr::col((LibraryTable::Table, LibraryTable::Id)).eq(id));
        Ok(Self::find(transaction, condition).await?.pop())
    }

    async fn find(
        transaction: &mut DbTransaction,
        condition: Condition,
    ) -> Result<Vec<Library>, DbError> {
        let (sql, values) = Query::select()
            .column(Asterisk)
            .from(LibraryTable::Table)
            .cond_where(condition.clone())
            .build_sqlx(SqliteQueryBuilder);
        let rows = sqlx::query_as_with::<_, LibraryRow, _>(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let (sql, values) = Query::select()
            .columns([(E::Table, E::LibraryId), (E::Table, E::Exclusion)])
            .from(E::Table)
            .inner_join(
                LibraryTable::Table,
                Expr::col((E::Table, E::LibraryId))
                    .eq(Expr::col((LibraryTable::Table, LibraryTable::Id))),
            )
            .cond_where(condition)
            .build_sqlx(SqliteQueryBuilder);
        let mut exclusions = BTreeMap::<LibraryId, BTreeSet<String>>::new();
        for (id, exclusion) in
            sqlx::query_as_with::<_, (LibraryId, String), _>(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?
        {
            exclusions.entry(id).or_default().insert(exclusion);
        }
        rows.into_iter()
            .map(|row| {
                let directory_exclusions = exclusions.remove(&row.id).unwrap_or_default();
                row.into_library(directory_exclusions)
            })
            .collect()
    }

    pub async fn insert(transaction: &mut DbTransaction, library: &Library) -> Result<(), DbError> {
        let fields = row::values(library);
        let (sql, values) = Query::insert()
            .into_table(LibraryTable::Table)
            .columns(
                std::iter::once(LibraryTable::Id).chain(fields.iter().map(|(column, _)| *column)),
            )
            .values_panic(
                std::iter::once(Expr::val(&library.id))
                    .chain(fields.into_iter().map(|(_, value)| value)),
            )
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Self::insert_exclusions(transaction, library).await
    }

    pub async fn update(transaction: &mut DbTransaction, library: &Library) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(LibraryTable::Table)
            .values(row::values(library))
            .value(LibraryTable::LastModifiedDate, Expr::current_timestamp())
            .and_where(Expr::col(LibraryTable::Id).eq(&library.id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        let (sql, values) = Query::delete()
            .from_table(E::Table)
            .and_where(Expr::col(E::LibraryId).eq(&library.id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Self::insert_exclusions(transaction, library).await
    }

    async fn insert_exclusions(
        transaction: &mut DbTransaction,
        library: &Library,
    ) -> Result<(), DbError> {
        for exclusion in &library.scan_directory_exclusions {
            let (sql, values) = Query::insert()
                .into_table(E::Table)
                .columns([E::LibraryId, E::Exclusion])
                .values_panic([(&library.id).into(), exclusion.as_str().into()])
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    /// Returns the subset of `ids` naming existing libraries.
    pub async fn find_existing_ids(
        transaction: &mut DbTransaction,
        ids: &BTreeSet<LibraryId>,
    ) -> Result<BTreeSet<LibraryId>, DbError> {
        if ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        let (sql, values) = Query::select()
            .column(LibraryTable::Id)
            .from(LibraryTable::Table)
            .and_where(Expr::col(LibraryTable::Id).is_in(ids))
            .build_sqlx(SqliteQueryBuilder);

        let existing: Vec<LibraryId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok(existing.into_iter().collect())
    }
}
