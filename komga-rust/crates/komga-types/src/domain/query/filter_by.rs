//! Optional narrowing of referential queries by an owning entity.

use std::collections::BTreeSet;

use crate::ids::{LibraryId, ReadListId, SeriesId};

/// Kotlin `FilterBy`.
///
/// Sets are non-empty; an empty request parameter means no filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterBy {
    Libraries(BTreeSet<LibraryId>),
    Collections(BTreeSet<String>),
    Series(BTreeSet<SeriesId>),
    ReadLists(BTreeSet<ReadListId>),
}

/// Kotlin `FilterTags`: which tags a tag listing covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterTags {
    Series,
    Book,
    Both,
}
