//! Kotlin `LuceneEntity`.

/// The kinds of entity the full-text index holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchEntity {
    Book,
    Series,
    Collection,
    ReadList,
}

impl SearchEntity {
    pub const ALL: [Self; 4] = [Self::Book, Self::Series, Self::Collection, Self::ReadList];

    /// `LuceneEntity.type`.
    #[must_use]
    pub fn type_name(self) -> &'static str {
        match self {
            Self::Book => "book",
            Self::Series => "series",
            Self::Collection => "collection",
            Self::ReadList => "readlist",
        }
    }
}
