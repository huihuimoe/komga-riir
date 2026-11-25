//! Kotlin `SeriesMetadataDao`.

use std::collections::HashMap;
use std::slice;

use crate::iden::SeriesMetadataAlternateTitleTable as A;
use crate::iden::SeriesMetadataGenreTable as G;
use crate::iden::SeriesMetadataLinkTable as K;
use crate::iden::SeriesMetadataSharingTable as L;
use crate::iden::SeriesMetadataTable as D;
use crate::iden::SeriesMetadataTagTable as T;
use komga_types::{
    AlternateTitle, ReadingDirection, Series, SeriesId, SeriesMetadata, SeriesStatus,
    StringOperator, WebLink,
};
use sea_query::{Cond, Expr, ExprTrait, Iden, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::support::rows::{
    BATCH_CHUNK_SIZE, find_pairs_by_owners, find_values_by_owners, replace_rows,
};
use crate::support::{local_date_time, search_operator};
use crate::{DbError, DbTransaction};

pub struct SeriesMetadataDao;

/// The `SERIES_METADATA` columns of `MetadataRow`.
const COLUMNS: [D; 24] = [
    D::SeriesId,
    D::Status,
    D::StatusLock,
    D::Title,
    D::TitleLock,
    D::TitleSort,
    D::TitleSortLock,
    D::Summary,
    D::SummaryLock,
    D::ReadingDirection,
    D::ReadingDirectionLock,
    D::Publisher,
    D::PublisherLock,
    D::AgeRating,
    D::AgeRatingLock,
    D::Language,
    D::LanguageLock,
    D::GenresLock,
    D::TotalBookCount,
    D::TotalBookCountLock,
    D::TagsLock,
    D::SharingLabelsLock,
    D::LinksLock,
    D::AlternateTitlesLock,
];

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct MetadataRow {
    series_id: SeriesId,
    status: SeriesStatus,
    status_lock: bool,
    title: String,
    title_lock: bool,
    title_sort: String,
    title_sort_lock: bool,
    summary: String,
    summary_lock: bool,
    reading_direction: Option<ReadingDirection>,
    reading_direction_lock: bool,
    publisher: String,
    publisher_lock: bool,
    age_rating: Option<i32>,
    age_rating_lock: bool,
    language: String,
    language_lock: bool,
    genres_lock: bool,
    total_book_count: Option<i32>,
    total_book_count_lock: bool,
    tags_lock: bool,
    sharing_labels_lock: bool,
    links_lock: bool,
    alternate_titles_lock: bool,
}

impl MetadataRow {
    fn into_metadata(
        self,
        genres: Vec<String>,
        tags: Vec<String>,
        sharing_labels: Vec<String>,
        links: Vec<(String, String)>,
        titles: Vec<(String, String)>,
    ) -> SeriesMetadata {
        SeriesMetadata {
            series_id: self.series_id,
            status: self.status,
            status_lock: self.status_lock,
            title: self.title,
            title_lock: self.title_lock,
            title_sort: self.title_sort,
            title_sort_lock: self.title_sort_lock,
            summary: self.summary,
            summary_lock: self.summary_lock,
            reading_direction: self.reading_direction,
            reading_direction_lock: self.reading_direction_lock,
            publisher: self.publisher,
            publisher_lock: self.publisher_lock,
            age_rating: self.age_rating,
            age_rating_lock: self.age_rating_lock,
            language: self.language,
            language_lock: self.language_lock,
            genres,
            genres_lock: self.genres_lock,
            total_book_count: self.total_book_count,
            total_book_count_lock: self.total_book_count_lock,
            tags,
            tags_lock: self.tags_lock,
            sharing_labels,
            sharing_labels_lock: self.sharing_labels_lock,
            links: links
                .into_iter()
                .map(|(label, url)| WebLink { label, url })
                .collect(),
            links_lock: self.links_lock,
            alternate_titles: titles
                .into_iter()
                .map(|(label, title)| AlternateTitle { label, title })
                .collect(),
            alternate_titles_lock: self.alternate_titles_lock,
        }
    }
}

impl SeriesMetadataDao {
    /// `SeriesLifecycle.createSeries`: an `ONGOING` metadata titled after the series; the other
    /// columns take their defaults, which equal the Kotlin `SeriesMetadata` defaults.
    pub async fn insert_for_new_series(
        transaction: &mut DbTransaction,
        series: &Series,
    ) -> Result<(), DbError> {
        let title = series.name.trim();
        let (sql, values) = Query::insert()
            .into_table(D::Table)
            .columns([D::SeriesId, D::Status, D::Title, D::TitleSort])
            .values_panic([
                (&series.id).into(),
                "ONGOING".into(),
                title.into(),
                title.into(),
            ])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// The age rating and sharing labels of `findById`, which content restrictions check.
    pub async fn find_age_rating_and_sharing_labels(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<(Option<i32>, Vec<String>), DbError> {
        let (sql, values) = Query::select()
            .column(D::AgeRating)
            .from(D::Table)
            .and_where(Expr::col(D::SeriesId).eq(series_id))
            .build_sqlx(SqliteQueryBuilder);
        let age_rating: Option<i32> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?;
        let (sql, values) = Query::select()
            .column(L::Label)
            .from(L::Table)
            .and_where(Expr::col(L::SeriesId).eq(series_id))
            .build_sqlx(SqliteQueryBuilder);
        let labels = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok((age_rating, labels))
    }

    /// The language and reading direction of `findById`, which WebPub manifests show.
    pub async fn find_language_and_reading_direction(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<(String, Option<ReadingDirection>), DbError> {
        let (sql, values) = Query::select()
            .columns([D::Language, D::ReadingDirection])
            .from(D::Table)
            .and_where(Expr::col(D::SeriesId).eq(series_id))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?)
    }

    /// `findByIdOrNull`.
    /// A series titled one of `titles`, else one whose title contains one of them, compared as
    /// the `Is` and `Contains` title search conditions do; any one when several match.
    pub async fn find_series_id_by_titles(
        transaction: &mut DbTransaction,
        titles: &[String],
    ) -> Result<Option<SeriesId>, DbError> {
        let is = |value: &String| StringOperator::Is {
            value: value.clone(),
        };
        let contains = |value: &String| StringOperator::Contains {
            value: value.clone(),
        };
        for operator in [is, contains] {
            let condition = titles.iter().fold(Cond::any(), |condition, title| {
                condition.add(search_operator::string(
                    &operator(title),
                    Expr::col((D::Table, D::Title)),
                ))
            });
            let (sql, values) = Query::select()
                .column(D::SeriesId)
                .from(D::Table)
                .cond_where(condition)
                .limit(1)
                .build_sqlx(SqliteQueryBuilder);
            let id: Option<SeriesId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(transaction.connection())
                .await?;
            if id.is_some() {
                return Ok(id);
            }
        }
        Ok(None)
    }

    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<Option<SeriesMetadata>, DbError> {
        Ok(Self::find_by_ids(transaction, slice::from_ref(series_id))
            .await?
            .remove(series_id))
    }

    /// Batch version of [`Self::find_by_id`] returning metadata for all matching `series_ids`.
    pub async fn find_by_ids(
        transaction: &mut DbTransaction,
        series_ids: &[SeriesId],
    ) -> Result<HashMap<SeriesId, SeriesMetadata>, DbError> {
        let mut rows = Vec::with_capacity(series_ids.len());
        for chunk in series_ids.chunks(BATCH_CHUNK_SIZE) {
            let (sql, values) = Query::select()
                .columns(COLUMNS)
                .from(D::Table)
                .and_where(Expr::col(D::SeriesId).is_in(chunk))
                .build_sqlx(SqliteQueryBuilder);
            rows.extend(
                sqlx::query_as_with::<_, MetadataRow, _>(AssertSqlSafe(sql.as_str()), values)
                    .fetch_all(transaction.connection())
                    .await?,
            );
        }
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<SeriesId> = rows.iter().map(|row| row.series_id.clone()).collect();
        let mut genres =
            find_values_by_owners(transaction, G::Table, G::SeriesId, G::Genre, &ids).await?;
        let mut tags =
            find_values_by_owners(transaction, T::Table, T::SeriesId, T::Tag, &ids).await?;
        let mut labels =
            find_values_by_owners(transaction, L::Table, L::SeriesId, L::Label, &ids).await?;
        let mut links =
            find_pairs_by_owners(transaction, K::Table, K::SeriesId, [K::Label, K::Url], &ids)
                .await?;
        let mut titles = find_pairs_by_owners(
            transaction,
            A::Table,
            A::SeriesId,
            [A::Label, A::Title],
            &ids,
        )
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let id = row.series_id.clone();
                let meta = row.into_metadata(
                    genres.remove(&id).unwrap_or_default(),
                    tags.remove(&id).unwrap_or_default(),
                    labels.remove(&id).unwrap_or_default(),
                    links.remove(&id).unwrap_or_default(),
                    titles.remove(&id).unwrap_or_default(),
                );
                (id, meta)
            })
            .collect())
    }

    /// `update`: every column and the genres, tags, sharing labels, links, and alternate titles
    /// are replaced, stamping `LAST_MODIFIED_DATE`.
    pub async fn update(
        transaction: &mut DbTransaction,
        metadata: &SeriesMetadata,
    ) -> Result<(), DbError> {
        let m = metadata;
        let (sql, values) = Query::update()
            .table(D::Table)
            .values([
                (D::Status, m.status.into()),
                (D::StatusLock, m.status_lock.into()),
                (D::Title, m.title.as_str().into()),
                (D::TitleLock, m.title_lock.into()),
                (D::TitleSort, m.title_sort.as_str().into()),
                (D::TitleSortLock, m.title_sort_lock.into()),
                (D::Summary, m.summary.as_str().into()),
                (D::SummaryLock, m.summary_lock.into()),
                (D::ReadingDirection, m.reading_direction.into()),
                (D::ReadingDirectionLock, m.reading_direction_lock.into()),
                (D::Publisher, m.publisher.as_str().into()),
                (D::PublisherLock, m.publisher_lock.into()),
                (D::AgeRating, m.age_rating.into()),
                (D::AgeRatingLock, m.age_rating_lock.into()),
                (D::Language, m.language.as_str().into()),
                (D::LanguageLock, m.language_lock.into()),
                (D::GenresLock, m.genres_lock.into()),
                (D::TotalBookCount, m.total_book_count.into()),
                (D::TotalBookCountLock, m.total_book_count_lock.into()),
                (D::TagsLock, m.tags_lock.into()),
                (D::SharingLabelsLock, m.sharing_labels_lock.into()),
                (D::LinksLock, m.links_lock.into()),
                (D::AlternateTitlesLock, m.alternate_titles_lock.into()),
                (D::LastModifiedDate, local_date_time::now_utc().into()),
            ])
            .and_where(Expr::col(D::SeriesId).eq(&m.series_id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        replace_related_rows(transaction, metadata).await
    }
}

async fn replace_related_rows(
    transaction: &mut DbTransaction,
    m: &SeriesMetadata,
) -> Result<(), DbError> {
    let id = &m.series_id;
    replace_values(
        transaction,
        G::Table,
        G::SeriesId,
        [G::SeriesId, G::Genre],
        id,
        &m.genres,
    )
    .await?;
    replace_values(
        transaction,
        T::Table,
        T::SeriesId,
        [T::SeriesId, T::Tag],
        id,
        &m.tags,
    )
    .await?;
    replace_values(
        transaction,
        L::Table,
        L::SeriesId,
        [L::SeriesId, L::Label],
        id,
        &m.sharing_labels,
    )
    .await?;
    let links = m
        .links
        .iter()
        .map(|l| [id.into(), l.label.as_str().into(), l.url.as_str().into()]);
    replace_rows(
        transaction,
        K::Table,
        K::SeriesId,
        [K::SeriesId, K::Label, K::Url],
        id,
        links,
    )
    .await?;
    let titles = m
        .alternate_titles
        .iter()
        .map(|t| [id.into(), t.label.as_str().into(), t.title.as_str().into()]);
    replace_rows(
        transaction,
        A::Table,
        A::SeriesId,
        [A::SeriesId, A::Label, A::Title],
        id,
        titles,
    )
    .await
}

/// Replaces a series' rows of a `[SERIES_ID, value]` table with `values`.
async fn replace_values<I: Iden + 'static>(
    transaction: &mut DbTransaction,
    table: I,
    series_column: I,
    columns: [I; 2],
    series_id: &SeriesId,
    values: &[String],
) -> Result<(), DbError> {
    let rows = values
        .iter()
        .map(|value| [series_id.into(), value.as_str().into()]);
    replace_rows(transaction, table, series_column, columns, series_id, rows).await
}
