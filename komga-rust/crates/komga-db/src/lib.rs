//! SQLite connection, migrations, and data access for Komga.

mod connection;
mod dao;
mod error;
mod iden;
mod lock;
mod migration;
mod sqlite;
mod support;

pub use connection::{Database, DbTransaction, INITIAL_SCHEMA_VERSION};
pub use dao::*;
pub use error::DbError;
pub use lock::DirectoryLock;
pub use sqlite::collation::{SortLocale, SortRules, UnsupportedSortLocale};
pub use support::search_candidates::SearchCandidates;
