//! Kotlin `BookMetadataPatch` and `MetadataApplier`: what a provider found, and how it changes
//! stored metadata.

use jiff::civil::Date;
use komga_types::{BookAuthor, BookMetadata, NotableEntity, WebLink};

/// `None` fields were not found and leave the stored value as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub(in crate::metadata) struct BookMetadataPatch {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub number: Option<String>,
    pub number_sort: Option<f32>,
    pub release_date: Option<Date>,
    pub authors: Option<Vec<BookAuthor>>,
    pub notable_entities: Option<Vec<NotableEntity>>,
    pub isbn: Option<String>,
    pub links: Option<Vec<WebLink>>,
    pub tags: Option<Vec<String>>,
    pub read_lists: Vec<ReadListEntry>,
}

/// Kotlin `BookMetadataPatch.ReadListEntry`: a read list the book belongs to, at `number`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::metadata) struct ReadListEntry {
    pub name: String,
    pub number: Option<i32>,
}

/// `MetadataApplier.apply`: each found value replaces the stored one unless that field is locked.
pub(in crate::metadata) fn apply(
    patch: &BookMetadataPatch,
    metadata: BookMetadata,
) -> BookMetadata {
    let m = metadata;
    BookMetadata {
        title: unless_locked(m.title, patch.title.as_ref(), m.title_lock),
        summary: unless_locked(m.summary, patch.summary.as_ref(), m.summary_lock),
        number: unless_locked(m.number, patch.number.as_ref(), m.number_lock),
        number_sort: unless_locked(
            m.number_sort,
            patch.number_sort.as_ref(),
            m.number_sort_lock,
        ),
        release_date: unless_locked(
            m.release_date,
            patch.release_date.map(Some).as_ref(),
            m.release_date_lock,
        ),
        authors: unless_locked(m.authors, patch.authors.as_ref(), m.authors_lock),
        notable_entities: unless_locked(
            m.notable_entities,
            patch.notable_entities.as_ref(),
            m.notable_entities_lock,
        ),
        isbn: unless_locked(m.isbn, patch.isbn.as_ref(), m.isbn_lock),
        links: unless_locked(m.links, patch.links.as_ref(), m.links_lock),
        tags: unless_locked(m.tags, patch.tags.as_ref(), m.tags_lock),
        ..m
    }
    .normalized()
}

/// `MetadataApplier.getIfNotLocked`.
pub(super) fn unless_locked<T: Clone>(original: T, patched: Option<&T>, lock: bool) -> T {
    match patched {
        Some(value) if !lock => value.clone(),
        _ => original,
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use komga_types::{BookAuthor, BookId, BookMetadata};

    use super::{BookMetadataPatch, apply};

    fn metadata() -> BookMetadata {
        BookMetadata {
            book_id: BookId::from("BOOK".to_owned()),
            title: "File name".to_owned(),
            title_lock: false,
            summary: String::new(),
            summary_lock: false,
            number: "1".to_owned(),
            number_lock: false,
            number_sort: 1.0,
            number_sort_lock: false,
            release_date: Some(date(2000, 1, 1)),
            release_date_lock: false,
            authors: vec![BookAuthor::new("Old", "writer")],
            authors_lock: false,
            notable_entities: Vec::new(),
            notable_entities_lock: false,
            tags: vec!["kept".to_owned()],
            tags_lock: false,
            isbn: String::new(),
            isbn_lock: false,
            links: Vec::new(),
            links_lock: false,
        }
    }

    #[test]
    fn found_values_replace_unlocked_fields_only() {
        let original = BookMetadata {
            number_lock: true,
            authors_lock: true,
            ..metadata()
        };
        let patch = BookMetadataPatch {
            title: Some("  Title ".to_owned()),
            number: Some("7".to_owned()),
            number_sort: Some(7.0),
            authors: Some(Vec::new()),
            tags: Some(vec![" A ".to_owned(), "a".to_owned(), " ".to_owned()]),
            ..BookMetadataPatch::default()
        };
        assert_eq!(
            apply(&patch, original.clone()),
            BookMetadata {
                title: "Title".to_owned(),
                number_sort: 7.0,
                tags: vec!["a".to_owned()],
                ..original
            }
        );
    }

    #[test]
    fn missing_values_keep_the_stored_ones() {
        assert_eq!(apply(&BookMetadataPatch::default(), metadata()), metadata());
    }
}
