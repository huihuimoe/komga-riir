//! Kotlin `SeriesDtoDao`: series as one user sees them.

mod condition;
mod search;

use crate::iden::BookMetadataAggregationTable as A;
use crate::iden::ReadProgressSeriesTable as RS;
use crate::iden::SeriesMetadataTable as SD;
use crate::iden::SeriesTable as S;
use komga_types::{SeriesDetails, SeriesId, UserId};
use sea_query::{Alias, Expr, ExprTrait, Query, SelectStatement, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{COLUMNS, SeriesMetadataDao, SeriesRow};
use crate::dao::BookMetadataAggregationDao;
use crate::support::local_date_time::SqlDateTime;
use crate::{DbError, DbTransaction};

pub struct SeriesDtoDao;

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct DetailsRow {
    #[sqlx(flatten)]
    series: SeriesRow,
    series_created_date: SqlDateTime,
    series_last_modified_date: SqlDateTime,
    read_count: Option<i32>,
    in_progress_count: Option<i32>,
    metadata_created_date: SqlDateTime,
    metadata_last_modified_date: SqlDateTime,
    aggregation_created_date: SqlDateTime,
    aggregation_last_modified_date: SqlDateTime,
}

impl SeriesDtoDao {
    /// `findByIdOrNull(seriesId, userId)`.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        user_id: &UserId,
    ) -> Result<Option<SeriesDetails>, DbError> {
        let mut query = select_details(user_id);
        query.and_where(Expr::col((S::Table, S::Id)).eq(series_id));
        Ok(fetch(transaction, &query).await?.pop())
    }
}

/// The series, the dates of its metadata and book metadata aggregation, and the user's read
/// counts.
fn select_details(user_id: &UserId) -> SelectStatement {
    let mut query = Query::select();
    query.columns(COLUMNS.map(|column| (S::Table, column)));
    for (column, name) in [
        (Expr::col((S::Table, S::CreatedDate)), "SERIES_CREATED_DATE"),
        (
            Expr::col((S::Table, S::LastModifiedDate)),
            "SERIES_LAST_MODIFIED_DATE",
        ),
        (Expr::col((RS::Table, RS::ReadCount)), "READ_COUNT"),
        (
            Expr::col((RS::Table, RS::InProgressCount)),
            "IN_PROGRESS_COUNT",
        ),
        (
            Expr::col((SD::Table, SD::CreatedDate)),
            "METADATA_CREATED_DATE",
        ),
        (
            Expr::col((SD::Table, SD::LastModifiedDate)),
            "METADATA_LAST_MODIFIED_DATE",
        ),
        (
            Expr::col((A::Table, A::CreatedDate)),
            "AGGREGATION_CREATED_DATE",
        ),
        (
            Expr::col((A::Table, A::LastModifiedDate)),
            "AGGREGATION_LAST_MODIFIED_DATE",
        ),
    ] {
        query.expr_as(column, Alias::new(name));
    }
    query
        .from(S::Table)
        .inner_join(
            SD::Table,
            Expr::col((SD::Table, SD::SeriesId)).equals((S::Table, S::Id)),
        )
        .inner_join(
            A::Table,
            Expr::col((A::Table, A::SeriesId)).equals((S::Table, S::Id)),
        )
        .left_join(
            RS::Table,
            Expr::col((RS::Table, RS::SeriesId))
                .equals((S::Table, S::Id))
                .and(Expr::col((RS::Table, RS::UserId)).eq(user_id)),
        );
    query
}

async fn fetch(
    transaction: &mut DbTransaction,
    query: &SelectStatement,
) -> Result<Vec<SeriesDetails>, DbError> {
    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
    let rows: Vec<DetailsRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(transaction.connection())
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let series_ids: Vec<SeriesId> = rows.iter().map(|row| row.series.id.clone()).collect();
    let mut metadata_by_series = SeriesMetadataDao::find_by_ids(transaction, &series_ids).await?;
    let mut aggregation_by_series =
        BookMetadataAggregationDao::find_by_ids(transaction, &series_ids).await?;
    let mut details = Vec::with_capacity(rows.len());
    for row in rows {
        let series = row.series.into_series()?;
        let metadata = metadata_by_series
            .remove(&series.id)
            .expect("The query joins the metadata of the series");
        let books_metadata = aggregation_by_series
            .remove(&series.id)
            .expect("The query joins the book metadata aggregation of the series");
        details.push(SeriesDetails {
            series,
            created: row.series_created_date.0,
            last_modified: row.series_last_modified_date.0,
            books_read_count: row.read_count.unwrap_or(0),
            books_in_progress_count: row.in_progress_count.unwrap_or(0),
            metadata,
            metadata_created: row.metadata_created_date.0,
            metadata_last_modified: row.metadata_last_modified_date.0,
            books_metadata,
            books_metadata_created: row.aggregation_created_date.0,
            books_metadata_last_modified: row.aggregation_last_modified_date.0,
        });
    }
    Ok(details)
}
