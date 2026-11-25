//! DAO for `HISTORICAL_EVENT` and its properties.

use std::collections::BTreeMap;

use crate::iden::{HistoricalEventPropertiesTable as P, HistoricalEventTable as E};
use komga_types::{
    BookId, HistoricalEvent, HistoricalEventRecord, HistoricalEventSort, PageRequest, PageResult,
    SeriesId, SortOrder,
};
use sea_query::{Asterisk, Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::sqlite::SqliteRow;
use sqlx::{AssertSqlSafe, Row};

use crate::connection::{Database, DbTransaction};
use crate::error::DbError;
use crate::support::local_date_time::{self, SqlDateTime};
use crate::support::rows::fetch_page;

pub struct HistoricalEventDao;

impl HistoricalEventDao {
    pub async fn insert(
        transaction: &mut DbTransaction,
        event: &HistoricalEvent,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(E::Table)
            .columns([E::Id, E::Type, E::BookId, E::SeriesId, E::Timestamp])
            .values_panic([
                event.id.clone().into(),
                event.kind.into(),
                event.book_id.as_ref().map(BookId::as_str).into(),
                event.series_id.as_ref().map(SeriesId::as_str).into(),
                local_date_time::now_utc().into(),
            ])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        if event.properties.is_empty() {
            return Ok(());
        }
        let mut insert = Query::insert();
        insert
            .into_table(P::Table)
            .columns([P::Id, P::Key, P::Value]);
        for (key, value) in &event.properties {
            insert.values_panic([event.id.clone().into(), (*key).into(), value.clone().into()]);
        }
        let (sql, values) = insert.build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// The requested page of events, with their properties.
    pub async fn find_page(
        database: &Database,
        sort: &[SortOrder<HistoricalEventSort>],
        page: PageRequest,
    ) -> Result<PageResult<HistoricalEventRecord>, DbError> {
        let mut query = Query::select();
        query.column(Asterisk).from(E::Table);
        let orders = sort
            .iter()
            .map(|order| (Expr::col(sort_column(order.key)), order.direction));
        let mut result = fetch_page(database, query, orders, page, from_row).await?;
        if result.items.is_empty() {
            return Ok(result);
        }
        let (sql, values) = Query::select()
            .columns([P::Id, P::Key, P::Value])
            .from(P::Table)
            .and_where(Expr::col(P::Id).is_in(result.items.iter().map(|event| event.id.as_str())))
            .build_sqlx(SqliteQueryBuilder);
        let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(database.pool())
            .await?;
        let mut properties: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for row in rows {
            properties
                .entry(row.try_get("ID")?)
                .or_default()
                .insert(row.try_get("KEY")?, row.try_get("VALUE")?);
        }
        for event in &mut result.items {
            event.properties = properties.remove(&event.id).unwrap_or_default();
        }
        Ok(result)
    }
}

fn sort_column(key: HistoricalEventSort) -> E {
    match key {
        HistoricalEventSort::Type => E::Type,
        HistoricalEventSort::BookId => E::BookId,
        HistoricalEventSort::SeriesId => E::SeriesId,
        HistoricalEventSort::Timestamp => E::Timestamp,
    }
}

fn from_row(row: &SqliteRow) -> Result<HistoricalEventRecord, DbError> {
    Ok(HistoricalEventRecord {
        id: row.try_get("ID")?,
        kind: row.try_get("TYPE")?,
        timestamp: row.try_get::<SqlDateTime, _>("TIMESTAMP")?.0,
        book_id: row.try_get("BOOK_ID")?,
        series_id: row.try_get("SERIES_ID")?,
        properties: BTreeMap::new(),
    })
}
