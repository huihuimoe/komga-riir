//! Historical event tables.

use sea_query::Iden;

/// `HISTORICAL_EVENT` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "HISTORICAL_EVENT"]
pub enum HistoricalEventTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "TYPE"]
    Type,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "TIMESTAMP"]
    Timestamp,
}

/// `HISTORICAL_EVENT_PROPERTIES` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "HISTORICAL_EVENT_PROPERTIES"]
pub enum HistoricalEventPropertiesTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "KEY"]
    Key,
    #[iden = "VALUE"]
    Value,
}
