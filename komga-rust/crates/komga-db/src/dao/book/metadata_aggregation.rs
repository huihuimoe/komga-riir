//! Kotlin `BookMetadataAggregationDao`.

use std::collections::HashMap;
use std::slice;

use crate::iden::BookMetadataAggregationAuthorTable as A;
use crate::iden::BookMetadataAggregationNotableEntitiesTable as N;
use crate::iden::BookMetadataAggregationTable as D;
use crate::iden::BookMetadataAggregationTagTable as T;
use komga_types::{BookAuthor, BookMetadataAggregation, NotableEntity, SeriesId};
use sea_query::{Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::support::local_date_time::{self, SqlDate};
use crate::support::rows::{
    BATCH_CHUNK_SIZE, find_pairs_by_owners, find_values_by_owners, replace_rows,
};
use crate::{DbError, DbTransaction};

pub struct BookMetadataAggregationDao;

impl BookMetadataAggregationDao {
    /// `insert(BookMetadataAggregation(seriesId))`: an empty aggregation.
    pub async fn insert_empty(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(D::Table)
            .columns([D::SeriesId])
            .values_panic([series_id.into()])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `findByIdOrNull`, with the authors, notable entities, and tags in table order.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<Option<BookMetadataAggregation>, DbError> {
        Ok(Self::find_by_ids(transaction, slice::from_ref(series_id))
            .await?
            .remove(series_id))
    }

    /// Batch version of [`Self::find_by_id`] returning aggregations for all matching `series_ids`.
    pub async fn find_by_ids(
        transaction: &mut DbTransaction,
        series_ids: &[SeriesId],
    ) -> Result<HashMap<SeriesId, BookMetadataAggregation>, DbError> {
        let rows = fetch_rows(transaction, series_ids).await?;
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<SeriesId> = rows.iter().map(|(id, _, _, _)| id.clone()).collect();
        let mut authors =
            find_pairs_by_owners(transaction, A::Table, A::SeriesId, [A::Name, A::Role], &ids)
                .await?;
        let mut entities =
            find_pairs_by_owners(transaction, N::Table, N::SeriesId, [N::Kind, N::Name], &ids)
                .await?;
        let mut tags =
            find_values_by_owners(transaction, T::Table, T::SeriesId, T::Tag, &ids).await?;

        Ok(rows
            .into_iter()
            .map(|(series_id, release_date, summary, summary_number)| {
                let agg = BookMetadataAggregation {
                    authors: authors
                        .remove(&series_id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(name, role)| BookAuthor { name, role })
                        .collect(),
                    notable_entities: entities
                        .remove(&series_id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(kind, name)| NotableEntity { kind, name })
                        .collect(),
                    tags: tags.remove(&series_id).unwrap_or_default(),
                    release_date: release_date.map(|date| date.0),
                    summary,
                    summary_number,
                    series_id: series_id.clone(),
                };
                (series_id, agg)
            })
            .collect())
    }

    /// `update`: every column and the authors, notable entities, and tags are replaced.
    pub async fn update(
        transaction: &mut DbTransaction,
        aggregation: &BookMetadataAggregation,
    ) -> Result<(), DbError> {
        let id = &aggregation.series_id;
        let (sql, values) = Query::update()
            .table(D::Table)
            .values([
                (D::Summary, aggregation.summary.as_str().into()),
                (D::SummaryNumber, aggregation.summary_number.as_str().into()),
                (D::ReleaseDate, aggregation.release_date.map(SqlDate).into()),
                (D::LastModifiedDate, local_date_time::now_utc().into()),
            ])
            .and_where(Expr::col(D::SeriesId).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;

        let authors = aggregation.authors.iter().map(|author| {
            [
                id.into(),
                author.name.as_str().into(),
                author.role.as_str().into(),
            ]
        });
        replace_rows(
            transaction,
            A::Table,
            A::SeriesId,
            [A::SeriesId, A::Name, A::Role],
            id,
            authors,
        )
        .await?;
        let entities = aggregation.notable_entities.iter().map(|entity| {
            [
                id.into(),
                entity.kind.as_str().into(),
                entity.name.as_str().into(),
            ]
        });
        replace_rows(
            transaction,
            N::Table,
            N::SeriesId,
            [N::SeriesId, N::Kind, N::Name],
            id,
            entities,
        )
        .await?;
        let tags = aggregation
            .tags
            .iter()
            .map(|tag| [id.into(), tag.as_str().into()]);
        replace_rows(
            transaction,
            T::Table,
            T::SeriesId,
            [T::SeriesId, T::Tag],
            id,
            tags,
        )
        .await
    }
}

type AggregationRow = (SeriesId, Option<SqlDate>, String, String);

async fn fetch_rows(
    transaction: &mut DbTransaction,
    series_ids: &[SeriesId],
) -> Result<Vec<AggregationRow>, DbError> {
    let mut rows = Vec::with_capacity(series_ids.len());
    for chunk in series_ids.chunks(BATCH_CHUNK_SIZE) {
        let (sql, values) = Query::select()
            .columns([D::SeriesId, D::ReleaseDate, D::Summary, D::SummaryNumber])
            .from(D::Table)
            .and_where(Expr::col(D::SeriesId).is_in(chunk))
            .build_sqlx(SqliteQueryBuilder);
        rows.extend(
            sqlx::query_as_with::<_, AggregationRow, _>(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?,
        );
    }
    Ok(rows)
}
