//! Kotlin `KoboDtoDao`: what Kobo devices are told about books.

use crate::iden::BookMetadataAuthorTable as A;
use crate::iden::BookMetadataTable as D;
use crate::iden::BookTable as B;
use crate::iden::MediaTable as M;
use crate::iden::ReadProgressTable as R;
use crate::iden::SeriesMetadataTable as SD;
use crate::iden::ThumbnailBookTable as T;
use jiff::civil::DateTime;
use komga_types::{BookId, KoboBook, KoboReadProgress, SeriesId, ThumbnailId, UserId};
use sea_query::{Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;
use tracing::warn;

use super::MediaDao;
use crate::support::json_gz;
use crate::support::local_date_time::{SqlDate, SqlDateTime};
use crate::{DbError, DbTransaction};

/// `(book, created, last modified, completed, compressed locator)`.
type ReadProgressRow = (BookId, SqlDateTime, SqlDateTime, bool, Option<Vec<u8>>);

pub struct KoboDao;

#[derive(sqlx::FromRow)]
struct KoboBookRow {
    id: BookId,
    title: String,
    number: String,
    number_sort: f32,
    isbn: String,
    summary: String,
    release_date: Option<SqlDate>,
    created_date: SqlDateTime,
    series_id: SeriesId,
    series_title: String,
    publisher: String,
    language: String,
    oneshot: bool,
    file_size: i64,
    epub_is_kepub: bool,
    cover: Option<ThumbnailId>,
}

impl KoboDao {
    /// `findBookMetadataByIds`, in no particular order.
    pub async fn find_books(
        transaction: &mut DbTransaction,
        ids: &[BookId],
    ) -> Result<Vec<KoboBook>, DbError> {
        let (sql, values) = Query::select()
            .expr_as(Expr::col((B::Table, B::Id)), "id")
            .expr_as(Expr::col((D::Table, D::Title)), "title")
            .expr_as(Expr::col((D::Table, D::Number)), "number")
            .expr_as(Expr::col((D::Table, D::NumberSort)), "number_sort")
            .expr_as(Expr::col((D::Table, D::Isbn)), "isbn")
            .expr_as(Expr::col((D::Table, D::Summary)), "summary")
            .expr_as(Expr::col((D::Table, D::ReleaseDate)), "release_date")
            .expr_as(Expr::col((D::Table, D::CreatedDate)), "created_date")
            .expr_as(Expr::col((SD::Table, SD::SeriesId)), "series_id")
            .expr_as(Expr::col((SD::Table, SD::Title)), "series_title")
            .expr_as(Expr::col((SD::Table, SD::Publisher)), "publisher")
            .expr_as(Expr::col((SD::Table, SD::Language)), "language")
            .expr_as(Expr::col((B::Table, B::Oneshot)), "oneshot")
            .expr_as(Expr::col((B::Table, B::FileSize)), "file_size")
            .expr_as(Expr::col((M::Table, M::EpubIsKepub)), "epub_is_kepub")
            .expr_as(Expr::col((T::Table, T::Id)), "cover")
            .from(B::Table)
            .inner_join(
                D::Table,
                Expr::col((D::Table, D::BookId)).equals((B::Table, B::Id)),
            )
            .inner_join(
                SD::Table,
                Expr::col((SD::Table, SD::SeriesId)).equals((B::Table, B::SeriesId)),
            )
            .inner_join(
                M::Table,
                Expr::col((M::Table, M::BookId)).equals((B::Table, B::Id)),
            )
            .left_join(
                T::Table,
                Expr::col((T::Table, T::BookId))
                    .equals((B::Table, B::Id))
                    .and(Expr::col((T::Table, T::Selected)).eq(true)),
            )
            .and_where(Expr::col((B::Table, B::Id)).is_in(ids))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<KoboBookRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        let mut books = Vec::with_capacity(rows.len());
        for row in rows {
            let is_fixed_layout = MediaDao::find_extension(transaction, &row.id)
                .await?
                .is_some_and(|extension| extension.is_fixed_layout);
            let authors = find_author_names(transaction, &row.id).await?;
            books.push(KoboBook {
                title: row.title,
                number: row.number,
                number_sort: row.number_sort,
                isbn: row.isbn,
                summary: row.summary,
                release_date: row.release_date.map(|date| date.0),
                metadata_created: row.created_date.0,
                series_id: row.series_id,
                series_title: row.series_title,
                publisher: row.publisher,
                language: row.language,
                oneshot: row.oneshot,
                file_size: row.file_size,
                is_kepub: row.epub_is_kepub,
                is_fixed_layout,
                cover: row.cover,
                authors,
                id: row.id,
            });
        }
        Ok(books)
    }

    /// The user's progress on `ids`, in no particular order; a locator that does not decode is
    /// dropped.
    pub async fn find_read_progress(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        ids: &[BookId],
    ) -> Result<Vec<KoboReadProgress>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                R::BookId,
                R::CreatedDate,
                R::LastModifiedDate,
                R::Completed,
                R::Locator,
            ])
            .from(R::Table)
            .and_where(Expr::col(R::UserId).eq(user_id))
            .and_where(Expr::col(R::BookId).is_in(ids))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<ReadProgressRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|(book_id, created, last_modified, completed, locator)| {
                let locator = locator.and_then(|blob| {
                    json_gz::deserialize(&blob)
                        .inspect_err(|error| warn!(book = %book_id, %error, "Cannot read locator"))
                        .ok()
                });
                KoboReadProgress {
                    book_id,
                    created: created.0,
                    last_modified: last_modified.0,
                    completed,
                    locator,
                }
            })
            .collect())
    }

    /// When the book was added, for the reading state of an unread book.
    pub async fn find_book_created(
        transaction: &mut DbTransaction,
        id: &BookId,
    ) -> Result<Option<DateTime>, DbError> {
        let (sql, values) = Query::select()
            .column(B::CreatedDate)
            .from(B::Table)
            .and_where(Expr::col(B::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        let created: Option<SqlDateTime> =
            sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(transaction.connection())
                .await?;
        Ok(created.map(|created| created.0))
    }
}

async fn find_author_names(
    transaction: &mut DbTransaction,
    book_id: &BookId,
) -> Result<Vec<String>, DbError> {
    let (sql, values) = Query::select()
        .column(A::Name)
        .from(A::Table)
        .and_where(Expr::col(A::BookId).eq(book_id))
        .build_sqlx(SqliteQueryBuilder);
    Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(transaction.connection())
        .await?)
}
