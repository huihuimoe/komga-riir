//! Full-text search over Tantivy with the analysis and query syntax of Komga's Lucene index.

mod analyzer;
mod document;
mod index;
mod query;
mod schema;

pub use analyzer::NgramOptions;
pub use document::{IndexUpdate, SearchDocument};
pub use index::{IndexGeneration, RebuildReason, SearchError, SearchHits, SearchIndex};
