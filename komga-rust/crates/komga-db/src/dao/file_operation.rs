//! `ERA_FILE_OPERATION`: files being written next to or over library files.

use crate::iden::FileOperationTable as T;
use komga_types::FileUrl;
use sea_query::{Expr, ExprTrait, Order, Query, SelectStatement, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::{DbError, DbTransaction};

pub struct FileOperationDao;

/// The size and modification time of a file: evidence, not proof, that its content is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileVersion {
    pub size: i64,
    /// Nanoseconds since the Unix epoch.
    pub modified: i64,
}

/// A file written from `source` to `target`, recorded before it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOperation {
    pub id: i64,
    pub source: FileUrl,
    pub source_version: FileVersion,
    pub target: FileUrl,
    /// A file the output supersedes: set aside when the output is published, removed once it is
    /// committed, and put back otherwise.
    pub replaced: Option<FileUrl>,
    /// Whether the source is removed once the output is committed.
    pub remove_source: bool,
    /// The output once verified, recorded before it is published.
    pub output_version: Option<FileVersion>,
    /// Whether the database changes that use the published output are committed.
    pub committed: bool,
}

type Row = (
    i64,
    String,
    i64,
    i64,
    String,
    Option<String>,
    bool,
    Option<i64>,
    Option<i64>,
    bool,
);

impl FileOperationDao {
    /// Records `operation`, whose ID, output and commitment are ignored; returns its ID.
    pub async fn insert(
        transaction: &mut DbTransaction,
        operation: &FileOperation,
    ) -> Result<i64, DbError> {
        let (sql, values) = Query::insert()
            .into_table(T::Table)
            .columns([
                T::SourceUrl,
                T::SourceSize,
                T::SourceModified,
                T::TargetUrl,
                T::ReplacedUrl,
                T::RemoveSource,
            ])
            .values_panic([
                operation.source.as_str().into(),
                operation.source_version.size.into(),
                operation.source_version.modified.into(),
                operation.target.as_str().into(),
                operation.replaced.as_ref().map(FileUrl::as_str).into(),
                operation.remove_source.into(),
            ])
            .returning_col(T::Id)
            .build_sqlx(SqliteQueryBuilder);
        let (id,): (i64,) = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?;
        Ok(id)
    }

    /// Records the verified output of operation `id`.
    pub async fn set_output(
        transaction: &mut DbTransaction,
        id: i64,
        version: FileVersion,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(T::Table)
            .value(T::OutputSize, version.size)
            .value(T::OutputModified, version.modified)
            .and_where(Expr::col(T::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Records that the database changes using the output of operation `id` are committed with
    /// `transaction`.
    pub async fn set_committed(transaction: &mut DbTransaction, id: i64) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(T::Table)
            .value(T::Committed, true)
            .and_where(Expr::col(T::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Every recorded operation, oldest first.
    pub async fn find_all(transaction: &mut DbTransaction) -> Result<Vec<FileOperation>, DbError> {
        let (sql, values) = select()
            .order_by(T::Id, Order::Asc)
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<Row> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok(rows.into_iter().map(operation).collect())
    }

    pub async fn find(
        transaction: &mut DbTransaction,
        id: i64,
    ) -> Result<Option<FileOperation>, DbError> {
        let (sql, values) = select()
            .and_where(Expr::col(T::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        let row: Option<Row> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(row.map(operation))
    }

    pub async fn delete(transaction: &mut DbTransaction, id: i64) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(T::Table)
            .and_where(Expr::col(T::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}

fn select() -> SelectStatement {
    let mut select = Query::select();
    select
        .columns([
            T::Id,
            T::SourceUrl,
            T::SourceSize,
            T::SourceModified,
            T::TargetUrl,
            T::ReplacedUrl,
            T::RemoveSource,
            T::OutputSize,
            T::OutputModified,
            T::Committed,
        ])
        .from(T::Table);
    select
}

fn operation(row: Row) -> FileOperation {
    let (
        id,
        source,
        source_size,
        source_modified,
        target,
        replaced,
        remove_source,
        output_size,
        output_modified,
        committed,
    ) = row;
    FileOperation {
        id,
        source: FileUrl::from(source),
        source_version: FileVersion {
            size: source_size,
            modified: source_modified,
        },
        target: FileUrl::from(target),
        replaced: replaced.map(FileUrl::from),
        remove_source,
        output_version: output_size
            .zip(output_modified)
            .map(|(size, modified)| FileVersion { size, modified }),
        committed,
    }
}
