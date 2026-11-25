//! Kotlin `SidecarDao`.

use crate::iden::SidecarTable as S;
use jiff::civil::DateTime;
use komga_types::{FileUrl, LibraryId, Sidecar};
use sea_query::{Expr, ExprTrait, OnConflict, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::support::local_date_time::SqlDateTime;
use crate::{DbError, DbTransaction};

/// Bound parameters per `DELETE`, well below SQLite's variable limit.
const DELETE_CHUNK: usize = 1000;

pub struct SidecarDao;

/// Kotlin `SidecarStored`, without the parent URL no caller reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSidecar {
    pub url: FileUrl,
    pub last_modified_time: DateTime,
    pub library_id: LibraryId,
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct SidecarRow {
    url: FileUrl,
    last_modified_time: SqlDateTime,
    library_id: LibraryId,
}

impl SidecarDao {
    /// `findAll`, across libraries.
    pub async fn find_all(transaction: &mut DbTransaction) -> Result<Vec<StoredSidecar>, DbError> {
        let (sql, values) = Query::select()
            .columns([S::Url, S::LastModifiedTime, S::LibraryId])
            .from(S::Table)
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<SidecarRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoredSidecar {
                    url: row.url,
                    last_modified_time: row.last_modified_time.0,
                    library_id: row.library_id,
                })
            })
            .collect()
    }

    /// `save`: inserts, or moves the sidecar with this URL to the new parent, time, and library.
    pub async fn save(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        sidecar: &Sidecar,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(S::Table)
            .columns([S::Url, S::ParentUrl, S::LastModifiedTime, S::LibraryId])
            .values_panic([
                (&sidecar.url).into(),
                (&sidecar.parent_url).into(),
                SqlDateTime(sidecar.last_modified_time).into(),
                library_id.into(),
            ])
            .on_conflict(
                OnConflict::column(S::Url)
                    .update_columns([S::LastModifiedTime, S::ParentUrl, S::LibraryId])
                    .to_owned(),
            )
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `deleteByLibraryIdAndUrls`.
    pub async fn delete_by_library_and_urls(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        urls: &[&FileUrl],
    ) -> Result<(), DbError> {
        for chunk in urls.chunks(DELETE_CHUNK) {
            let (sql, values) = Query::delete()
                .from_table(S::Table)
                .and_where(Expr::col(S::LibraryId).eq(library_id))
                .and_where(Expr::col(S::Url).is_in(chunk.iter().copied()))
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }
}
