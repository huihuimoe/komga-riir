//! SeaQuery `Iden` symbols for Komga database schema.

// Complete 1:1 schema mirror verified by `migration::iden_sync`; some columns are only populated
// by SQL defaults and not directly constructed in non-test queries.
#![allow(dead_code)]

mod book;
mod event;
mod file_operation;
mod library;
mod search;
mod series;
mod server;
mod set;
mod task;
mod user;

pub use book::*;
pub use event::*;
pub use file_operation::*;
pub use library::*;
pub use search::*;
pub use series::*;
pub use server::*;
pub use set::*;
pub use task::*;
pub use user::*;
