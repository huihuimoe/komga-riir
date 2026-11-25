//! What the index holds: one document per book, series, collection, and read list.

use komga_types::SearchEntity;

/// The fields a query term without `field:` searches (`LuceneEntity.defaultFields`).
pub(crate) fn default_fields(entity: SearchEntity) -> &'static [&'static str] {
    match entity {
        SearchEntity::Book => &["title", "isbn"],
        SearchEntity::Series => &["title"],
        SearchEntity::Collection | SearchEntity::ReadList => &["name"],
    }
}

/// The searchable text of one entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchDocument {
    pub(crate) entity: SearchEntity,
    pub(crate) id: String,
    pub(crate) fields: Vec<(String, String)>,
}

impl SearchDocument {
    pub fn new(entity: SearchEntity, id: String) -> Self {
        Self {
            entity,
            id,
            fields: Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Adds a value to a field. A field may hold several values, and any name is accepted:
    /// each author role names its own field.
    pub fn add(&mut self, field: &str, value: &str) {
        self.fields.push((field.to_owned(), value.to_owned()));
    }
}

/// A change to apply to the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexUpdate {
    /// Adds the document, replacing any previous one of the same entity.
    Upsert(SearchDocument),
    Delete {
        entity: SearchEntity,
        id: String,
    },
}
