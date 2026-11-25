//! What Kotlin `BookDto.toDocument` and `SeriesDto.oneshotDocument` read of a book.

use std::collections::HashSet;

use crate::iden::{
    BookMetadataAuthorTable as A, BookMetadataTable as D, BookMetadataTagTable as T,
    BookTable as B, MediaTable as M, SeriesMetadataGenreTable as G,
    SeriesMetadataSharingTable as S, SeriesMetadataTable as SM,
};
use jiff::civil::Date;
use komga_types::BookAuthor;
use sea_query::{Expr, ExprTrait, JoinType, Query, SelectStatement, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{find_authors, find_values};
use crate::dao::BookDao;
use crate::support::local_date_time::SqlDate;
use crate::{DbError, DbTransaction};

/// The searchable state of a book, soft-deleted or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSearchFields {
    pub id: String,
    pub title: String,
    pub isbn: String,
    /// Distinct, in table order.
    pub tags: Vec<String>,
    pub authors: Vec<BookAuthor>,
    pub release_date: Option<Date>,
    pub media_status: String,
    pub deleted: bool,
    /// The metadata of its series, which a oneshot's document carries too.
    pub oneshot: Option<OneshotSeriesFields>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneshotSeriesFields {
    pub publisher: String,
    pub status: String,
    pub reading_direction: String,
    pub age_rating: Option<i32>,
    pub language: String,
    pub genres: Vec<String>,
    pub sharing_labels: Vec<String>,
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct Row {
    id: String,
    series_id: String,
    title: String,
    isbn: String,
    release_date: Option<SqlDate>,
    status: String,
    deleted: bool,
    /// Only a oneshot's series metadata is joined.
    publisher: Option<String>,
    series_status: Option<String>,
    reading_direction: Option<String>,
    age_rating: Option<i32>,
    language: Option<String>,
}

impl BookDao {
    /// The searchable state of the books among `ids` that exist with their media and metadata.
    pub async fn find_search_fields(
        transaction: &mut DbTransaction,
        ids: &[&str],
    ) -> Result<Vec<BookSearchFields>, DbError> {
        let (sql, values) = select(ids).build_sqlx(SqliteQueryBuilder);
        let rows: Vec<Row> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;

        let mut authors =
            find_authors(transaction, A::Table, A::BookId, [A::Name, A::Role], ids).await?;
        let mut tags = find_values(transaction, T::Table, T::BookId, T::Tag, ids).await?;
        let oneshot_series: Vec<&str> = rows
            .iter()
            .filter(|row| row.series_status.is_some())
            .map(|row| row.series_id.as_str())
            .collect();
        let genres = find_values(
            transaction,
            G::Table,
            G::SeriesId,
            G::Genre,
            &oneshot_series,
        )
        .await?;
        let sharing_labels = find_values(
            transaction,
            S::Table,
            S::SeriesId,
            S::Label,
            &oneshot_series,
        )
        .await?;

        rows.into_iter()
            .map(|row| {
                let oneshot = row.series_status.map(|status| OneshotSeriesFields {
                    publisher: row.publisher.unwrap_or_default(),
                    status,
                    reading_direction: row.reading_direction.unwrap_or_default(),
                    age_rating: row.age_rating,
                    language: row.language.unwrap_or_default(),
                    genres: genres.get(&row.series_id).cloned().unwrap_or_default(),
                    sharing_labels: sharing_labels
                        .get(&row.series_id)
                        .cloned()
                        .unwrap_or_default(),
                });
                let mut book_tags = tags.remove(&row.id).unwrap_or_default();
                let mut seen = HashSet::new();
                book_tags.retain(|tag| seen.insert(tag.clone()));
                Ok(BookSearchFields {
                    authors: authors.remove(&row.id).unwrap_or_default(),
                    tags: book_tags,
                    title: row.title,
                    isbn: row.isbn,
                    release_date: row.release_date.map(|date| date.0),
                    media_status: row.status,
                    deleted: row.deleted,
                    oneshot,
                    id: row.id,
                })
            })
            .collect()
    }
}

/// The book, metadata, and media columns, with the series metadata of a oneshot.
fn select(ids: &[&str]) -> SelectStatement {
    Query::select()
        .columns([(B::Table, B::Id), (B::Table, B::SeriesId)])
        .columns([
            (D::Table, D::Title),
            (D::Table, D::Isbn),
            (D::Table, D::ReleaseDate),
        ])
        .column((M::Table, M::Status))
        .expr_as(
            Expr::col((B::Table, B::DeletedDate)).is_not_null(),
            "DELETED",
        )
        .columns([
            (SM::Table, SM::Publisher),
            (SM::Table, SM::ReadingDirection),
            (SM::Table, SM::AgeRating),
            (SM::Table, SM::Language),
        ])
        .expr_as(Expr::col((SM::Table, SM::Status)), "SERIES_STATUS")
        .from(B::Table)
        .inner_join(
            D::Table,
            Expr::col((D::Table, D::BookId)).equals((B::Table, B::Id)),
        )
        .inner_join(
            M::Table,
            Expr::col((M::Table, M::BookId)).equals((B::Table, B::Id)),
        )
        .join(
            JoinType::LeftJoin,
            SM::Table,
            Expr::col((SM::Table, SM::SeriesId))
                .equals((B::Table, B::SeriesId))
                .and(Expr::col((B::Table, B::Oneshot)).eq(true)),
        )
        .and_where(Expr::col((B::Table, B::Id)).is_in(ids.iter().copied()))
        .take()
}
