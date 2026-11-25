//! `BookMetadataUpdateDto` and Kotlin `BookMetadata.patch(BookMetadataUpdateDto)`: an absent field
//! keeps the stored value; `null` keeps the title, number, number sort, and locks, and unsets the
//! other fields.

use jiff::civil::Date;
use komga_service::BookMetadataUpdate;
use komga_types::{BookAuthor, NotableEntity, WebLink};
use serde::Deserialize;

use crate::error::Violation;
use crate::rest::dto::present;
use crate::rest::validation;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadataUpdateDto {
    title: Option<String>,
    title_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    summary: Option<Option<String>>,
    summary_lock: Option<bool>,
    number: Option<String>,
    number_lock: Option<bool>,
    number_sort: Option<f32>,
    number_sort_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    release_date: Option<Option<Date>>,
    release_date_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    authors: Option<Option<Vec<AuthorUpdateDto>>>,
    authors_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    notable_entities: Option<Option<Vec<NotableEntityUpdateDto>>>,
    notable_entities_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    tags: Option<Option<Vec<String>>>,
    tags_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    isbn: Option<Option<String>>,
    isbn_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    links: Option<Option<Vec<WebLinkUpdateDto>>>,
    links_lock: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AuthorUpdateDto {
    name: Option<String>,
    role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NotableEntityUpdateDto {
    kind: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(in crate::rest::dto) struct WebLinkUpdateDto {
    label: Option<String>,
    url: Option<String>,
}

/// An ISO date that may be `null`, distinguished from an absent one.
/// `@NotBlank`, to which `null` is blank, on both fields of each element, named like Spring's
/// `list[index].field`.
pub(in crate::rest::dto) fn not_blank_pairs<'a>(
    list: &str,
    names: [&str; 2],
    pairs: impl Iterator<Item = [Option<&'a str>; 2]>,
    violations: &mut Vec<Violation>,
) {
    for (index, values) in pairs.enumerate() {
        for (name, value) in names.into_iter().zip(values) {
            let field_name = format!("{list}[{index}].{name}");
            validation::not_blank(field_name, value.unwrap_or_default(), violations);
        }
    }
}

impl BookMetadataUpdateDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::null_or_not_blank("title", self.title.as_deref(), &mut violations);
        validation::null_or_not_blank("number", self.number.as_deref(), &mut violations);
        if let Some(Some(authors)) = &self.authors {
            let pairs = authors
                .iter()
                .map(|author| [author.name.as_deref(), author.role.as_deref()]);
            not_blank_pairs("authors", ["name", "role"], pairs, &mut violations);
        }
        if let Some(Some(entities)) = &self.notable_entities {
            let pairs = entities
                .iter()
                .map(|entity| [entity.kind.as_deref(), entity.name.as_deref()]);
            not_blank_pairs("notableEntities", ["kind", "name"], pairs, &mut violations);
        }
        let isbn = self.isbn.as_ref().and_then(Option::as_deref);
        validation::null_or_blank_or_isbn("isbn", isbn, &mut violations);
        if let Some(Some(links)) = &self.links {
            web_link_violations(links, &mut violations);
        }
        violations
    }

    pub(crate) fn into_update(self) -> BookMetadataUpdate {
        BookMetadataUpdate {
            title: self.title,
            title_lock: self.title_lock,
            summary: self.summary.map(Option::unwrap_or_default),
            summary_lock: self.summary_lock,
            number: self.number,
            number_lock: self.number_lock,
            number_sort: self.number_sort,
            number_sort_lock: self.number_sort_lock,
            release_date: self.release_date,
            release_date_lock: self.release_date_lock,
            authors: self.authors.map(|authors| {
                authors
                    .unwrap_or_default()
                    .into_iter()
                    .map(|author| BookAuthor::new(&validated(author.name), &validated(author.role)))
                    .collect()
            }),
            authors_lock: self.authors_lock,
            notable_entities: self.notable_entities.map(|entities| {
                entities
                    .unwrap_or_default()
                    .into_iter()
                    .map(|entity| {
                        NotableEntity::new(&validated(entity.kind), &validated(entity.name))
                    })
                    .collect()
            }),
            notable_entities_lock: self.notable_entities_lock,
            tags: self.tags.map(Option::unwrap_or_default),
            tags_lock: self.tags_lock,
            isbn: self.isbn.map(|isbn| {
                isbn.unwrap_or_default()
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect()
            }),
            isbn_lock: self.isbn_lock,
            links: self.links.map(|links| web_links(links.unwrap_or_default())),
            links_lock: self.links_lock,
        }
    }
}

/// `@NotBlank` labels and `@URL` URLs of `links[index]`.
pub(in crate::rest::dto) fn web_link_violations(
    links: &[WebLinkUpdateDto],
    violations: &mut Vec<Violation>,
) {
    for (index, link) in links.iter().enumerate() {
        let label = link.label.as_deref().unwrap_or_default();
        validation::not_blank(format!("links[{index}].label"), label, violations);
        validation::url(
            format!("links[{index}].url"),
            link.url.as_deref(),
            violations,
        );
    }
}

pub(in crate::rest::dto) fn web_links(links: Vec<WebLinkUpdateDto>) -> Vec<WebLink> {
    links
        .into_iter()
        .map(|link| WebLink {
            label: validated(link.label),
            url: validated(link.url),
        })
        .collect()
}

pub(in crate::rest::dto) fn validated(value: Option<String>) -> String {
    value.expect("Validation rejects a missing value")
}
