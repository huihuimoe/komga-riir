//! WebPub publication metadata mapping (`toWPMetadataDto` and `withAuthors`).

use jiff::tz::TimeZone;
use komga_types::{BookAuthor, BookDetails};

use super::dto::{WpBelongsTo, WpContributor, WpLink, WpMetadata};
use crate::mvc::jackson_date_time;

/// `wpKnownRoles`; other roles are contributors.
const KNOWN_ROLES: [&str; 10] = [
    "author",
    "translator",
    "editor",
    "artist",
    "illustrator",
    "letterer",
    "penciler",
    "penciller",
    "colorist",
    "inker",
];

/// `toWPMetadataDto` and `withAuthors`.
pub(super) fn metadata(details: &BookDetails, series_links: Vec<WpLink>) -> WpMetadata {
    let book = &details.metadata;
    let role = |role: &str| -> Vec<String> {
        book.authors
            .iter()
            .filter(|author| author.role == role)
            .map(|author| author.name.clone())
            .collect()
    };
    let mut penciler = role("penciler");
    penciler.extend(role("penciller"));
    let contributor = book
        .authors
        .iter()
        .filter(|author: &&BookAuthor| !KNOWN_ROLES.contains(&author.role.as_str()))
        .map(|author| author.name.clone())
        .collect();
    WpMetadata {
        title: book.title.clone(),
        identifier: (!book.isbn.trim().is_empty()).then(|| format!("urn:isbn:{}", book.isbn)),
        conforms_to: None,
        // `toZonedDateTime()`: the server's zone.
        modified: jackson_date_time::iso_offset_date_time(
            &details
                .last_modified
                .to_zoned_datetime(TimeZone::UTC)
                .expect("Stored dates are in range")
                .with_time_zone(TimeZone::system()),
        ),
        published: book.release_date,
        language: String::new(),
        author: role("author"),
        translator: role("translator"),
        editor: role("editor"),
        artist: role("artist"),
        illustrator: role("illustrator"),
        letterer: role("letterer"),
        penciler,
        colorist: role("colorist"),
        inker: role("inker"),
        contributor,
        subject: book.tags.clone(),
        reading_progression: None,
        description: book.summary.clone(),
        number_of_pages: details.media.page_count,
        belongs_to: WpBelongsTo {
            series: vec![WpContributor {
                name: details.series_title.clone(),
                position: book.number_sort,
                links: series_links,
            }],
        },
        rendition: None,
    }
}
