//! Domain types grouped by aggregate.

pub(crate) mod book;
mod catalog;
mod event;
pub(crate) mod identity;
mod library;
mod primitives;
mod query;
mod series;
mod server;
mod sync;

pub use book::*;
pub use catalog::*;
pub use event::{
    BookImport, BookProgressRef, BookRef, CollectionRef, DomainEvent, ReadListRef,
    SeriesProgressRef, SeriesRef, SetId, ThumbnailRef,
};
pub use identity::*;
pub use library::*;
pub use primitives::*;
pub use query::*;
pub use series::*;
pub use server::*;
pub use sync::*;
