//! The Tantivy fields behind Komga's Lucene fields.

use std::collections::HashMap;
use std::ops::Bound;

use tantivy::Term;
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
};

/// Every field name Komga's documents use besides author roles. Each gets an index field, so
/// its term statistics and lengths are its own, as in Lucene.
const NAMED_FIELDS: &[&str] = &[
    "title",
    "isbn",
    "name",
    "tag",
    "series_tag",
    "book_tag",
    "author",
    "release_date",
    "status",
    "deleted",
    "oneshot",
    "publisher",
    "reading_direction",
    "age_rating",
    "language",
    "genre",
    "sharing_label",
    "total_book_count",
    "book_count",
    "complete",
];

/// Holds every other field (author roles), each term prefixed with its field name and a NUL.
const OTHER_FIELDS: &str = "other";

const SEPARATOR: char = '\0';

pub(crate) struct SearchSchema {
    pub(crate) schema: Schema,
    /// `<type>:<id>`, unique per document.
    pub(crate) key: Field,
    pub(crate) entity_type: Field,
    pub(crate) id: Field,
    named: HashMap<&'static str, Field>,
    other: Field,
}

impl SearchSchema {
    pub(crate) fn new() -> Self {
        let mut builder = Schema::builder();
        let key = builder.add_text_field("key", STRING);
        let entity_type = builder.add_text_field("type", STRING);
        let id = builder.add_text_field("id", STORED);
        // Terms come pre-analyzed, so the tokenizer name is never used.
        let text = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default().set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );
        let named = NAMED_FIELDS
            .iter()
            .map(|&name| (name, builder.add_text_field(name, text.clone())))
            .collect();
        let other = builder.add_text_field(OTHER_FIELDS, text);
        Self {
            schema: builder.build(),
            key,
            entity_type,
            id,
            named,
            other,
        }
    }

    /// Where the terms of a Lucene field live. Any name is a field; one no document uses
    /// matches nothing.
    pub(crate) fn target(&self, name: &str) -> FieldTarget {
        match self.named.get(name) {
            Some(&field) => FieldTarget {
                field,
                prefix: String::new(),
            },
            None => FieldTarget {
                field: self.other,
                prefix: format!("{name}{SEPARATOR}"),
            },
        }
    }
}

pub(crate) struct FieldTarget {
    pub(crate) field: Field,
    /// Precedes every term of the field in the index field.
    pub(crate) prefix: String,
}

impl FieldTarget {
    pub(crate) fn term_text(&self, text: &str) -> String {
        format!("{}{text}", self.prefix)
    }

    pub(crate) fn term(&self, text: &str) -> Term {
        Term::from_field_text(self.field, &self.term_text(text))
    }

    /// The bound past the last term of this field.
    pub(crate) fn upper_limit(&self) -> Bound<Term> {
        match self.prefix.strip_suffix(SEPARATOR) {
            Some(name) => {
                Bound::Excluded(Term::from_field_text(self.field, &format!("{name}\u{1}")))
            }
            None => Bound::Unbounded,
        }
    }
}
