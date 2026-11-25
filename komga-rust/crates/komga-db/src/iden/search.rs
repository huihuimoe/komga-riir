//! The komga-rust table (`era` migrations) of pending search index updates.

use sea_query::Iden;

/// `ERA_SEARCH_INDEX_PENDING`: entities whose index document must be brought up to date.
#[derive(Iden, Clone, Copy)]
#[iden = "ERA_SEARCH_INDEX_PENDING"]
pub enum SearchIndexPendingTable {
    Table,
    #[iden = "SEQ"]
    Seq,
    #[iden = "ENTITY"]
    Entity,
    #[iden = "ENTITY_ID"]
    EntityId,
}
