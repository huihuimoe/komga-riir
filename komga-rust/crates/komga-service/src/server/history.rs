//! Historical events listing (`HistoricalEventController`).

use komga_db::{Database, DbError, HistoricalEventDao};
use komga_types::{HistoricalEventRecord, HistoricalEventSort, PageRequest, PageResult, SortOrder};

/// Lists the recorded historical events.
#[derive(Clone)]
pub struct HistoryService {
    db: Database,
}

impl HistoryService {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// The requested page of events, with their properties.
    pub async fn events(
        &self,
        sort: &[SortOrder<HistoricalEventSort>],
        page: PageRequest,
    ) -> Result<PageResult<HistoricalEventRecord>, DbError> {
        HistoricalEventDao::find_page(&self.db, sort, page).await
    }
}
