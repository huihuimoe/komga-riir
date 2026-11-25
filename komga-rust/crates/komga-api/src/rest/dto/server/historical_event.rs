//! `HistoricalEventDto`.

use std::collections::BTreeMap;

use komga_types::HistoricalEventRecord;
use serde::Serialize;

use crate::rest::dto::JsonDateTime;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalEventDto {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    timestamp: JsonDateTime,
    book_id: Option<String>,
    series_id: Option<String>,
    properties: BTreeMap<String, String>,
}

impl From<HistoricalEventRecord> for HistoricalEventDto {
    fn from(event: HistoricalEventRecord) -> Self {
        Self {
            id: event.id,
            kind: event.kind,
            timestamp: JsonDateTime(event.timestamp),
            book_id: event.book_id,
            series_id: event.series_id,
            properties: event.properties,
        }
    }
}
