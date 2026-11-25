//! Kotlin `SeriesDto`, `SeriesMetadataDto`, and `BookMetadataAggregationDto`.

use jiff::civil::Date;
use std::path::PathBuf;

use komga_types::{AlternateTitle, LibraryId, SeriesDetails, SeriesId};
use serde::Serialize;

use crate::rest::dto::{AuthorDto, JsonDateTime, NotableEntityDto, WebLinkDto, file_time_in_utc};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDto {
    id: SeriesId,
    library_id: LibraryId,
    name: String,
    url: String,
    created: JsonDateTime,
    last_modified: JsonDateTime,
    file_last_modified: JsonDateTime,
    books_count: i32,
    books_read_count: i32,
    books_unread_count: i32,
    books_in_progress_count: i32,
    metadata: SeriesMetadataDto,
    books_metadata: BookMetadataAggregationDto,
    deleted: bool,
    oneshot: bool,
}

impl SeriesDto {
    /// `restrictUrl(!isAdmin)`: only admins see the path of the directory.
    #[must_use]
    pub fn new(details: SeriesDetails, admin: bool) -> Self {
        let books_unread_count = details.books_unread_count();
        let SeriesDetails {
            series,
            created,
            last_modified,
            books_read_count,
            books_in_progress_count,
            metadata,
            metadata_created,
            metadata_last_modified,
            books_metadata,
            books_metadata_created,
            books_metadata_last_modified,
        } = details;
        let url = if admin {
            // Like Java's `Path`, without the trailing slash of a directory URL.
            let path: PathBuf = series
                .url
                .to_path()
                .expect("Series have file URLs")
                .components()
                .collect();
            path.to_string_lossy().into_owned()
        } else {
            String::new()
        };
        Self {
            id: series.id,
            library_id: series.library_id,
            name: series.name,
            url,
            created: JsonDateTime(created),
            last_modified: JsonDateTime(last_modified),
            file_last_modified: JsonDateTime(file_time_in_utc(series.file_last_modified)),
            books_count: series.book_count,
            books_read_count,
            books_unread_count,
            books_in_progress_count,
            metadata: SeriesMetadataDto {
                status: metadata.status.as_str(),
                status_lock: metadata.status_lock,
                title: metadata.title,
                title_lock: metadata.title_lock,
                title_sort: metadata.title_sort,
                title_sort_lock: metadata.title_sort_lock,
                summary: metadata.summary,
                summary_lock: metadata.summary_lock,
                reading_direction: metadata
                    .reading_direction
                    .map_or("", |direction| direction.as_str()),
                reading_direction_lock: metadata.reading_direction_lock,
                publisher: metadata.publisher,
                publisher_lock: metadata.publisher_lock,
                age_rating: metadata.age_rating,
                age_rating_lock: metadata.age_rating_lock,
                language: metadata.language,
                language_lock: metadata.language_lock,
                genres: metadata.genres,
                genres_lock: metadata.genres_lock,
                tags: metadata.tags,
                tags_lock: metadata.tags_lock,
                total_book_count: metadata.total_book_count,
                total_book_count_lock: metadata.total_book_count_lock,
                sharing_labels: metadata.sharing_labels,
                sharing_labels_lock: metadata.sharing_labels_lock,
                links: metadata.links.into_iter().map(WebLinkDto::from).collect(),
                links_lock: metadata.links_lock,
                alternate_titles: metadata
                    .alternate_titles
                    .into_iter()
                    .map(AlternateTitleDto::from)
                    .collect(),
                alternate_titles_lock: metadata.alternate_titles_lock,
                created: JsonDateTime(metadata_created),
                last_modified: JsonDateTime(metadata_last_modified),
            },
            books_metadata: BookMetadataAggregationDto {
                authors: books_metadata
                    .authors
                    .into_iter()
                    .map(AuthorDto::from)
                    .collect(),
                notable_entities: books_metadata
                    .notable_entities
                    .into_iter()
                    .map(NotableEntityDto::from)
                    .collect(),
                tags: books_metadata.tags,
                release_date: books_metadata.release_date,
                summary: books_metadata.summary,
                summary_number: books_metadata.summary_number,
                created: JsonDateTime(books_metadata_created),
                last_modified: JsonDateTime(books_metadata_last_modified),
            },
            deleted: series.deleted_date.is_some(),
            oneshot: series.oneshot,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SeriesMetadataDto {
    status: &'static str,
    status_lock: bool,
    title: String,
    title_lock: bool,
    title_sort: String,
    title_sort_lock: bool,
    summary: String,
    summary_lock: bool,
    /// Empty when not set.
    reading_direction: &'static str,
    reading_direction_lock: bool,
    publisher: String,
    publisher_lock: bool,
    age_rating: Option<i32>,
    age_rating_lock: bool,
    language: String,
    language_lock: bool,
    genres: Vec<String>,
    genres_lock: bool,
    tags: Vec<String>,
    tags_lock: bool,
    total_book_count: Option<i32>,
    total_book_count_lock: bool,
    sharing_labels: Vec<String>,
    sharing_labels_lock: bool,
    links: Vec<WebLinkDto>,
    links_lock: bool,
    alternate_titles: Vec<AlternateTitleDto>,
    alternate_titles_lock: bool,
    created: JsonDateTime,
    last_modified: JsonDateTime,
}

#[derive(Debug, Serialize)]
struct AlternateTitleDto {
    label: String,
    title: String,
}

impl From<AlternateTitle> for AlternateTitleDto {
    fn from(title: AlternateTitle) -> Self {
        Self {
            label: title.label,
            title: title.title,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BookMetadataAggregationDto {
    authors: Vec<AuthorDto>,
    notable_entities: Vec<NotableEntityDto>,
    tags: Vec<String>,
    release_date: Option<Date>,
    summary: String,
    summary_number: String,
    created: JsonDateTime,
    last_modified: JsonDateTime,
}
