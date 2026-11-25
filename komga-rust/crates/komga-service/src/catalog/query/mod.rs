//! Browsing queries over visible content.

mod books;
mod series;
mod sets;

use komga_db::{Database, DbError};
use thiserror::Error;

use crate::catalog::search_index::{FullTextSearch, SearchIndexError};

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("Not found")]
    NotFound,
    /// `ContentRestrictionChecker` denied the item.
    #[error("Content restricted")]
    Restricted,
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error(transparent)]
    Search(#[from] SearchIndexError),
}

/// Catalog queries, always scoped to what the requesting user may see.
#[derive(Clone)]
pub struct CatalogService {
    db: Database,
    full_text: FullTextSearch,
}

impl CatalogService {
    #[must_use]
    pub fn new(db: Database, full_text: FullTextSearch) -> Self {
        Self { db, full_text }
    }
}
