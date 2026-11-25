//! Kotlin `MediaDao`.

use crate::iden::BookTable as B;
use crate::iden::MediaFileTable as F;
use crate::iden::MediaPageTable as P;
use crate::iden::MediaTable as M;
use jiff::civil::DateTime;
use komga_types::{
    BookId, BookPage, Dimension, LibraryId, Media, MediaExtensionEpub, MediaFile, MediaFileSubType,
    MediaStatus,
};
use sea_query::{Expr, ExprTrait, Order, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;
use tracing::warn;

use crate::support::json_gz;
use crate::support::local_date_time::{self, SqlDateTime};
use crate::{DbError, DbTransaction};

pub struct MediaDao;

/// Kotlin `MEDIA.EXTENSION_CLASS`, the qualified name of the extension class.
const EPUB_EXTENSION_CLASS: &str = "org.gotson.komga.domain.model.MediaExtensionEpub";

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct MediaRow {
    status: MediaStatus,
    media_type: Option<String>,
    page_count: i32,
    comment: Option<String>,
    epub_divina_compatible: bool,
    epub_is_kepub: bool,
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct PageRow {
    file_name: String,
    media_type: String,
    /// Declared lowercase by Kotlin's `V20200730135746__image_dimension.sql`.
    #[sqlx(rename = "width")]
    width: Option<i64>,
    #[sqlx(rename = "height")]
    height: Option<i64>,
    file_hash: String,
    file_size: Option<i64>,
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct FileRow {
    file_name: String,
    media_type: Option<String>,
    sub_type: Option<MediaFileSubType>,
    file_size: Option<i64>,
}

impl MediaDao {
    /// `insert(Media(bookId))`: an `UNKNOWN` media without pages or files; the other columns take
    /// their defaults, which equal the Kotlin `Media` defaults.
    pub async fn insert_unknown(
        transaction: &mut DbTransaction,
        book_ids: &[BookId],
    ) -> Result<(), DbError> {
        for book_id in book_ids {
            let (sql, values) = Query::insert()
                .into_table(M::Table)
                .columns([M::BookId, M::Status])
                .values_panic([book_id.into(), "UNKNOWN".into()])
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    /// Kotlin `update(findById(bookId).copy(status = status))`: the other columns, pages and
    /// files are written back unchanged, so only the status and modification date change.
    pub async fn update_status(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        status: &str,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(M::Table)
            .value(M::Status, status)
            .value(M::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(M::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `findByIdOrNull`: pages in page order, files in table order. Like Kotlin's
    /// `ProxyExtension`, the extension is not read, so updating the result keeps it.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        book_id: &BookId,
    ) -> Result<Option<Media>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                M::Status,
                M::MediaType,
                M::PageCount,
                M::Comment,
                M::EpubDivinaCompatible,
                M::EpubIsKepub,
            ])
            .from(M::Table)
            .and_where(Expr::col(M::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        let Some(row): Option<MediaRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?
        else {
            return Ok(None);
        };
        let status = row.status;

        let (sql, values) = Query::select()
            .columns([
                P::FileName,
                P::MediaType,
                P::Width,
                P::Height,
                P::FileHash,
                P::FileSize,
            ])
            .from(P::Table)
            .and_where(Expr::col(P::BookId).eq(book_id))
            .order_by(P::Number, Order::Asc)
            .build_sqlx(SqliteQueryBuilder);
        let pages: Vec<PageRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;

        let (sql, values) = Query::select()
            .columns([F::FileName, F::MediaType, F::SubType, F::FileSize])
            .from(F::Table)
            .and_where(Expr::col(F::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        let files: Vec<FileRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        let files = files
            .into_iter()
            .map(|file| MediaFile {
                file_name: file.file_name,
                media_type: file.media_type,
                sub_type: file.sub_type,
                file_size: file.file_size,
            })
            .collect();

        Ok(Some(Media {
            status,
            media_type: row.media_type,
            pages: pages.into_iter().map(PageRow::into_page).collect(),
            page_count: row.page_count,
            files,
            comment: row.comment,
            epub_divina_compatible: row.epub_divina_compatible,
            epub_is_kepub: row.epub_is_kepub,
            extension: None,
        }))
    }

    /// The distinct page media types of each of `book_ids` that has pages.
    pub async fn find_page_media_types(
        transaction: &mut DbTransaction,
        book_ids: &[BookId],
    ) -> Result<Vec<(BookId, String)>, DbError> {
        let (sql, values) = Query::select()
            .distinct()
            .columns([P::BookId, P::MediaType])
            .from(P::Table)
            .and_where(Expr::col(P::BookId).is_in(book_ids))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// Kotlin `Media.lastModifiedDate`, in UTC.
    pub async fn find_last_modified_date(
        transaction: &mut DbTransaction,
        book_id: &BookId,
    ) -> Result<Option<DateTime>, DbError> {
        let (sql, values) = Query::select()
            .column(M::LastModifiedDate)
            .from(M::Table)
            .and_where(Expr::col(M::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        let value: Option<SqlDateTime> =
            sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(transaction.connection())
                .await?;
        Ok(value.map(|date| date.0))
    }

    /// Kotlin `findExtensionByIdOrNull`: `None` without a stored EPUB extension, and, as Kotlin
    /// swallows deserialization errors, for a blob that does not decode.
    pub async fn find_extension(
        transaction: &mut DbTransaction,
        book_id: &BookId,
    ) -> Result<Option<MediaExtensionEpub>, DbError> {
        let (sql, values) = Query::select()
            .columns([M::ExtensionClass, M::ExtensionValueBlob])
            .from(M::Table)
            .and_where(Expr::col(M::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        let row: Option<(Option<String>, Option<Vec<u8>>)> =
            sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(transaction.connection())
                .await?;
        let Some((Some(class), Some(blob))) = row else {
            return Ok(None);
        };
        if class != EPUB_EXTENSION_CLASS {
            return Ok(None);
        }
        let extension = json_gz::deserialize(&blob)
            .inspect_err(|error| warn!(book = %book_id, %error, "Cannot read media extension"))
            .ok();
        Ok(extension)
    }

    /// Kotlin `findAllBookIdsByLibraryIdAndMediaTypeAndWithMissingPageHash`: ready books of
    /// `media_types` in the library, deleted or not, with fewer hashed pages than the first and
    /// last `page_hashing` pages, or all pages when the book has fewer.
    pub async fn find_book_ids_with_missing_page_hash(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        media_types: &[&str],
        page_hashing: i32,
    ) -> Result<Vec<BookId>, DbError> {
        let (sql, values) = Query::select()
            .column((B::Table, B::Id))
            .from(B::Table)
            .left_join(
                P::Table,
                Expr::col((B::Table, B::Id)).equals((P::Table, P::BookId)),
            )
            .left_join(
                M::Table,
                Expr::col((B::Table, B::Id)).equals((M::Table, M::BookId)),
            )
            .and_where(Expr::col((B::Table, B::LibraryId)).eq(library_id))
            .and_where(Expr::col((M::Table, M::Status)).eq(MediaStatus::Ready.as_str()))
            .and_where(Expr::col((M::Table, M::MediaType)).is_in(media_types.iter().copied()))
            .group_by_col((B::Table, B::Id))
            .and_having(Expr::cust_with_values(
                r#"SUM(CASE WHEN "MEDIA_PAGE"."FILE_HASH" = '' THEN 0 ELSE 1 END)
                    < MIN(COUNT("MEDIA_PAGE"."BOOK_ID"), ?)"#,
                [page_hashing * 2],
            ))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// Kotlin `update(media)`: pages and files are replaced, `LAST_MODIFIED_DATE` is stamped, and
    /// the extension columns change only when the media has an extension.
    pub async fn update(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        media: &Media,
    ) -> Result<(), DbError> {
        let mut update = Query::update();
        if let Some(extension) = &media.extension {
            update
                .value(M::ExtensionClass, EPUB_EXTENSION_CLASS)
                .value(M::ExtensionValueBlob, json_gz::serialize(extension));
        }
        let (sql, values) = update
            .table(M::Table)
            .value(M::Status, media.status)
            .value(M::MediaType, media.media_type.as_deref())
            .value(M::Comment, media.comment.as_deref())
            .value(M::PageCount, media.page_count)
            .value(M::EpubDivinaCompatible, media.epub_divina_compatible)
            .value(M::EpubIsKepub, media.epub_is_kepub)
            .value(M::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(M::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;

        let deletes = [
            Query::delete()
                .from_table(P::Table)
                .and_where(Expr::col(P::BookId).eq(book_id))
                .take(),
            Query::delete()
                .from_table(F::Table)
                .and_where(Expr::col(F::BookId).eq(book_id))
                .take(),
        ];
        for delete in deletes {
            let (sql, values) = delete.build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }

        for (number, page) in (0_i32..).zip(&media.pages) {
            let (sql, values) = Query::insert()
                .into_table(P::Table)
                .columns([
                    P::BookId,
                    P::FileName,
                    P::MediaType,
                    P::Number,
                    P::Width,
                    P::Height,
                    P::FileHash,
                    P::FileSize,
                ])
                .values_panic([
                    book_id.into(),
                    page.file_name.as_str().into(),
                    page.media_type.as_str().into(),
                    number.into(),
                    page.dimension.map(|dimension| dimension.width).into(),
                    page.dimension.map(|dimension| dimension.height).into(),
                    page.file_hash.as_str().into(),
                    page.file_size.into(),
                ])
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }

        for file in &media.files {
            let (sql, values) = Query::insert()
                .into_table(F::Table)
                .columns([
                    F::BookId,
                    F::FileName,
                    F::MediaType,
                    F::SubType,
                    F::FileSize,
                ])
                .values_panic([
                    book_id.into(),
                    file.file_name.as_str().into(),
                    file.media_type.as_deref().into(),
                    file.sub_type.into(),
                    file.file_size.into(),
                ])
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }
}

impl PageRow {
    /// Kotlin keeps a dimension only when both sides are stored.
    fn into_page(self) -> BookPage {
        let side = |value: Option<i64>| value.and_then(|side| u32::try_from(side).ok());
        let dimension = match (side(self.width), side(self.height)) {
            (Some(width), Some(height)) => Some(Dimension { width, height }),
            _ => None,
        };
        BookPage {
            file_name: self.file_name,
            media_type: self.media_type,
            dimension,
            file_hash: self.file_hash,
            file_size: self.file_size,
        }
    }
}

#[cfg(test)]
mod tests;
