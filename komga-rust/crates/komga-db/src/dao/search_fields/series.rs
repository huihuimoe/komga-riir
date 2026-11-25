//! What Kotlin `SeriesDto.toDocument` reads of a series.

use std::collections::HashMap;

use crate::iden::{
    BookMetadataAggregationAuthorTable as AA, BookMetadataAggregationTable as BA,
    BookMetadataAggregationTagTable as AT, SeriesMetadataAlternateTitleTable as AL,
    SeriesMetadataGenreTable as G, SeriesMetadataSharingTable as SH, SeriesMetadataTable as D,
    SeriesMetadataTagTable as T, SeriesTable as S,
};
use jiff::civil::Date;
use komga_types::BookAuthor;
use sea_query::{Expr, ExprTrait, JoinType, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{find_authors, find_values};
use crate::dao::SeriesDao;
use crate::support::local_date_time::SqlDate;
use crate::{DbError, DbTransaction};

/// The searchable state of a series, soft-deleted or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesSearchFields {
    pub id: String,
    pub title: String,
    pub title_sort: String,
    pub alternate_titles: Vec<String>,
    pub publisher: String,
    pub status: String,
    pub reading_direction: String,
    pub age_rating: Option<i32>,
    pub language: String,
    pub tags: Vec<String>,
    pub genres: Vec<String>,
    pub sharing_labels: Vec<String>,
    pub total_book_count: Option<i32>,
    pub book_count: i32,
    /// The aggregation of its books' metadata.
    pub book_tags: Vec<String>,
    pub book_authors: Vec<BookAuthor>,
    pub book_release_date: Option<Date>,
    pub deleted: bool,
    pub oneshot: bool,
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct Row {
    id: String,
    book_count: i32,
    deleted: bool,
    #[sqlx(rename = "oneshot")]
    oneshot: bool,
    title: String,
    title_sort: String,
    publisher: String,
    status: String,
    reading_direction: Option<String>,
    age_rating: Option<i32>,
    language: String,
    total_book_count: Option<i32>,
    release_date: Option<SqlDate>,
}

impl SeriesDao {
    /// The searchable state of the series among `ids` that exist with their metadata.
    pub async fn find_search_fields(
        transaction: &mut DbTransaction,
        ids: &[&str],
    ) -> Result<Vec<SeriesSearchFields>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                (S::Table, S::Id),
                (S::Table, S::BookCount),
                (S::Table, S::Oneshot),
            ])
            .expr_as(
                Expr::col((S::Table, S::DeletedDate)).is_not_null(),
                "DELETED",
            )
            .columns([
                (D::Table, D::Title),
                (D::Table, D::TitleSort),
                (D::Table, D::Publisher),
                (D::Table, D::Status),
                (D::Table, D::ReadingDirection),
                (D::Table, D::AgeRating),
                (D::Table, D::Language),
                (D::Table, D::TotalBookCount),
            ])
            .column((BA::Table, BA::ReleaseDate))
            .from(S::Table)
            .inner_join(
                D::Table,
                Expr::col((D::Table, D::SeriesId)).equals((S::Table, S::Id)),
            )
            .join(
                JoinType::LeftJoin,
                BA::Table,
                Expr::col((BA::Table, BA::SeriesId)).equals((S::Table, S::Id)),
            )
            .and_where(Expr::col((S::Table, S::Id)).is_in(ids.iter().copied()))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<Row> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;

        let mut alternate_titles =
            find_values(transaction, AL::Table, AL::SeriesId, AL::Title, ids).await?;
        let mut tags = find_values(transaction, T::Table, T::SeriesId, T::Tag, ids).await?;
        let mut genres = find_values(transaction, G::Table, G::SeriesId, G::Genre, ids).await?;
        let mut sharing_labels =
            find_values(transaction, SH::Table, SH::SeriesId, SH::Label, ids).await?;
        let mut book_tags = find_values(transaction, AT::Table, AT::SeriesId, AT::Tag, ids).await?;
        let mut book_authors = find_authors(
            transaction,
            AA::Table,
            AA::SeriesId,
            [AA::Name, AA::Role],
            ids,
        )
        .await?;

        rows.into_iter()
            .map(|row| {
                let take = |values: &mut HashMap<String, Vec<String>>| {
                    values.remove(&row.id).unwrap_or_default()
                };
                Ok(SeriesSearchFields {
                    alternate_titles: take(&mut alternate_titles),
                    tags: take(&mut tags),
                    genres: take(&mut genres),
                    sharing_labels: take(&mut sharing_labels),
                    book_tags: take(&mut book_tags),
                    book_authors: book_authors.remove(&row.id).unwrap_or_default(),
                    book_release_date: row.release_date.map(|date| date.0),
                    title: row.title,
                    title_sort: row.title_sort,
                    publisher: row.publisher,
                    status: row.status,
                    reading_direction: row.reading_direction.unwrap_or_default(),
                    age_rating: row.age_rating,
                    language: row.language,
                    total_book_count: row.total_book_count,
                    book_count: row.book_count,
                    deleted: row.deleted,
                    oneshot: row.oneshot,
                    id: row.id,
                })
            })
            .collect()
    }
}
