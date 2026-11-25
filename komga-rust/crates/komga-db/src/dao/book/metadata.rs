//! Kotlin `BookMetadataDao`.

use std::collections::{HashMap, HashSet};
use std::slice;

use crate::iden::BookMetadataAuthorTable as A;
use crate::iden::BookMetadataLinkTable as L;
use crate::iden::BookMetadataNotableEntitiesTable as N;
use crate::iden::BookMetadataTable as D;
use crate::iden::BookMetadataTagTable as T;
use komga_types::{Book, BookAuthor, BookId, BookMetadata, NotableEntity, WebLink};
use sea_query::{Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::support::local_date_time::{self, SqlDate};
use crate::support::rows::{
    BATCH_CHUNK_SIZE, find_pairs_by_owners, find_values_by_owners, replace_rows,
};
use crate::{DbError, DbTransaction};

pub struct BookMetadataDao;

/// The numbering part of a book's metadata.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct BookNumbering {
    pub book_id: BookId,
    pub number: String,
    pub number_lock: bool,
    pub number_sort: f32,
    pub number_sort_lock: bool,
}

const COLUMNS: [D; 17] = [
    D::BookId,
    D::Title,
    D::TitleLock,
    D::Summary,
    D::SummaryLock,
    D::Number,
    D::NumberLock,
    D::NumberSort,
    D::NumberSortLock,
    D::ReleaseDate,
    D::ReleaseDateLock,
    D::AuthorsLock,
    D::NotableEntitiesLock,
    D::TagsLock,
    D::Isbn,
    D::IsbnLock,
    D::LinksLock,
];

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct MetadataRow {
    book_id: BookId,
    title: String,
    title_lock: bool,
    summary: String,
    summary_lock: bool,
    number: String,
    number_lock: bool,
    number_sort: f32,
    number_sort_lock: bool,
    release_date: Option<SqlDate>,
    release_date_lock: bool,
    authors_lock: bool,
    notable_entities_lock: bool,
    tags_lock: bool,
    isbn: String,
    isbn_lock: bool,
    links_lock: bool,
}

impl MetadataRow {
    fn into_metadata(
        self,
        authors: Vec<(String, String)>,
        entities: Vec<(String, String)>,
        mut tags: Vec<String>,
        links: Vec<(String, String)>,
    ) -> BookMetadata {
        // Kotlin `toSet()` keeps the first of equal tags in table order.
        let mut seen = HashSet::with_capacity(tags.len());
        tags.retain(|tag| seen.insert(tag.clone()));
        BookMetadata {
            book_id: self.book_id,
            title: self.title,
            title_lock: self.title_lock,
            summary: self.summary,
            summary_lock: self.summary_lock,
            number: self.number,
            number_lock: self.number_lock,
            number_sort: self.number_sort,
            number_sort_lock: self.number_sort_lock,
            release_date: self.release_date.map(|date| date.0),
            release_date_lock: self.release_date_lock,
            authors: authors
                .into_iter()
                .map(|(name, role)| BookAuthor { name, role })
                .collect(),
            authors_lock: self.authors_lock,
            notable_entities: entities
                .into_iter()
                .map(|(kind, name)| NotableEntity { kind, name })
                .collect(),
            notable_entities_lock: self.notable_entities_lock,
            tags,
            tags_lock: self.tags_lock,
            isbn: self.isbn,
            isbn_lock: self.isbn_lock,
            links: links
                .into_iter()
                .map(|(label, url)| WebLink { label, url })
                .collect(),
            links_lock: self.links_lock,
        }
    }
}

impl BookMetadataDao {
    /// `findByIdOrNull`.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        book_id: &BookId,
    ) -> Result<Option<BookMetadata>, DbError> {
        Ok(Self::find_by_ids(transaction, slice::from_ref(book_id))
            .await?
            .remove(book_id))
    }

    /// Batch version of [`Self::find_by_id`] returning metadata for all matching `book_ids`.
    pub async fn find_by_ids(
        transaction: &mut DbTransaction,
        book_ids: &[BookId],
    ) -> Result<HashMap<BookId, BookMetadata>, DbError> {
        let rows = fetch_rows(transaction, book_ids).await?;
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<BookId> = rows.iter().map(|row| row.book_id.clone()).collect();
        let mut authors =
            find_pairs_by_owners(transaction, A::Table, A::BookId, [A::Name, A::Role], &ids)
                .await?;
        let mut entities =
            find_pairs_by_owners(transaction, N::Table, N::BookId, [N::Kind, N::Name], &ids)
                .await?;
        let mut tags =
            find_values_by_owners(transaction, T::Table, T::BookId, T::Tag, &ids).await?;
        let mut links =
            find_pairs_by_owners(transaction, L::Table, L::BookId, [L::Label, L::Url], &ids)
                .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let id = row.book_id.clone();
                let meta = row.into_metadata(
                    authors.remove(&id).unwrap_or_default(),
                    entities.remove(&id).unwrap_or_default(),
                    tags.remove(&id).unwrap_or_default(),
                    links.remove(&id).unwrap_or_default(),
                );
                (id, meta)
            })
            .collect())
    }

    /// `update`: every column and the authors, notable entities, tags, and links are replaced.
    pub async fn update(
        transaction: &mut DbTransaction,
        metadata: &BookMetadata,
    ) -> Result<(), DbError> {
        let id = &metadata.book_id;
        let (sql, values) = Query::update()
            .table(D::Table)
            .values([
                (D::Title, metadata.title.as_str().into()),
                (D::TitleLock, metadata.title_lock.into()),
                (D::Summary, metadata.summary.as_str().into()),
                (D::SummaryLock, metadata.summary_lock.into()),
                (D::Number, metadata.number.as_str().into()),
                (D::NumberLock, metadata.number_lock.into()),
                (D::NumberSort, metadata.number_sort.into()),
                (D::NumberSortLock, metadata.number_sort_lock.into()),
                (D::ReleaseDate, metadata.release_date.map(SqlDate).into()),
                (D::ReleaseDateLock, metadata.release_date_lock.into()),
                (D::AuthorsLock, metadata.authors_lock.into()),
                (
                    D::NotableEntitiesLock,
                    metadata.notable_entities_lock.into(),
                ),
                (D::TagsLock, metadata.tags_lock.into()),
                (D::Isbn, metadata.isbn.as_str().into()),
                (D::IsbnLock, metadata.isbn_lock.into()),
                (D::LinksLock, metadata.links_lock.into()),
                (D::LastModifiedDate, local_date_time::now_utc().into()),
            ])
            .and_where(Expr::col(D::BookId).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;

        replace_related_rows(transaction, metadata).await
    }

    /// `SeriesLifecycle.addBooks`: metadata titled after the file and numbered after
    /// `Book.number`; the other columns take their defaults, which equal the Kotlin defaults.
    pub async fn insert_for_new_books(
        transaction: &mut DbTransaction,
        books: &[Book],
    ) -> Result<(), DbError> {
        for book in books {
            #[expect(
                clippy::cast_precision_loss,
                reason = "Kotlin `Int.toFloat()` rounds the same way"
            )]
            let number_sort = book.number as f32;
            let (sql, values) = Query::insert()
                .into_table(D::Table)
                .columns([D::BookId, D::Title, D::Number, D::NumberSort])
                .values_panic([
                    (&book.id).into(),
                    book.name.trim().into(),
                    book.number.to_string().into(),
                    number_sort.into(),
                ])
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    pub async fn find_numbering(
        transaction: &mut DbTransaction,
        book_ids: &[BookId],
    ) -> Result<Vec<BookNumbering>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                D::BookId,
                D::Number,
                D::NumberLock,
                D::NumberSort,
                D::NumberSortLock,
            ])
            .from(D::Table)
            .and_where(Expr::col(D::BookId).is_in(book_ids))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// `update` of a metadata whose numbering changed; the other columns keep their values.
    pub async fn update_numbering(
        transaction: &mut DbTransaction,
        numbering: &BookNumbering,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(D::Table)
            .value(D::Number, numbering.number.trim())
            .value(D::NumberSort, numbering.number_sort)
            .value(D::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(D::BookId).eq(&numbering.book_id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}

async fn fetch_rows(
    transaction: &mut DbTransaction,
    book_ids: &[BookId],
) -> Result<Vec<MetadataRow>, DbError> {
    let mut rows = Vec::with_capacity(book_ids.len());
    for chunk in book_ids.chunks(BATCH_CHUNK_SIZE) {
        let (sql, values) = Query::select()
            .columns(COLUMNS)
            .from(D::Table)
            .and_where(Expr::col(D::BookId).is_in(chunk))
            .build_sqlx(SqliteQueryBuilder);
        rows.extend(
            sqlx::query_as_with::<_, MetadataRow, _>(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?,
        );
    }
    Ok(rows)
}

/// Replaces the authors, notable entities, tags, and links of `metadata`'s book.
async fn replace_related_rows(
    transaction: &mut DbTransaction,
    metadata: &BookMetadata,
) -> Result<(), DbError> {
    let id = &metadata.book_id;
    let authors = metadata
        .authors
        .iter()
        .map(|a| [id.into(), a.name.as_str().into(), a.role.as_str().into()]);
    replace_rows(
        transaction,
        A::Table,
        A::BookId,
        [A::BookId, A::Name, A::Role],
        id,
        authors,
    )
    .await?;
    let entities = metadata
        .notable_entities
        .iter()
        .map(|e| [id.into(), e.name.as_str().into(), e.kind.as_str().into()]);
    replace_rows(
        transaction,
        N::Table,
        N::BookId,
        [N::BookId, N::Name, N::Kind],
        id,
        entities,
    )
    .await?;
    let tags = metadata.tags.iter().map(|t| [id.into(), t.as_str().into()]);
    replace_rows(
        transaction,
        T::Table,
        T::BookId,
        [T::BookId, T::Tag],
        id,
        tags,
    )
    .await?;
    let links = metadata
        .links
        .iter()
        .map(|l| [id.into(), l.label.as_str().into(), l.url.as_str().into()]);
    replace_rows(
        transaction,
        L::Table,
        L::BookId,
        [L::BookId, L::Label, L::Url],
        id,
        links,
    )
    .await
}
