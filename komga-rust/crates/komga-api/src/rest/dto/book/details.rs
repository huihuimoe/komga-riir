//! Kotlin `BookDto`, `MediaDto`, `BookMetadataDto`, and `ReadProgressDto`.

use jiff::civil::Date;
use komga_types::{
    BookAuthor, BookDetails, BookId, LibraryId, MediaDetails, MediaProfile, NotableEntity,
    ReadProgressDetails, SeriesId, WebLink,
};
use serde::Serialize;

use crate::mvc::media::format_size;
use crate::rest::dto::{JsonDateTime, file_time_in_utc};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDto {
    id: BookId,
    series_id: SeriesId,
    series_title: String,
    library_id: LibraryId,
    name: String,
    url: String,
    number: i32,
    created: JsonDateTime,
    last_modified: JsonDateTime,
    file_last_modified: JsonDateTime,
    size_bytes: i64,
    size: String,
    media: MediaDto,
    metadata: BookMetadataDto,
    read_progress: Option<ReadProgressDto>,
    deleted: bool,
    file_hash: String,
    oneshot: bool,
}

impl BookDto {
    /// `toDto().restrictUrl(!isAdmin)`: only admins see the full path of the file.
    #[must_use]
    pub fn new(details: BookDetails, admin: bool) -> Self {
        let BookDetails {
            book,
            series_title,
            created,
            last_modified,
            media,
            metadata,
            metadata_created,
            metadata_last_modified,
            read_progress,
        } = details;
        let path = book.url.to_path().expect("Books have file URLs");
        let url = if admin {
            path.to_string_lossy().into_owned()
        } else {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        Self {
            id: book.id,
            series_id: book.series_id,
            series_title,
            library_id: book.library_id,
            name: book.name,
            url,
            number: book.number,
            created: JsonDateTime(created),
            last_modified: JsonDateTime(last_modified),
            file_last_modified: JsonDateTime(file_time_in_utc(book.file_last_modified)),
            size_bytes: book.file_size,
            size: format_size(book.file_size),
            media: media.into(),
            metadata: BookMetadataDto {
                title: metadata.title,
                title_lock: metadata.title_lock,
                summary: metadata.summary,
                summary_lock: metadata.summary_lock,
                number: metadata.number,
                number_lock: metadata.number_lock,
                number_sort: metadata.number_sort,
                number_sort_lock: metadata.number_sort_lock,
                release_date: metadata.release_date,
                release_date_lock: metadata.release_date_lock,
                authors: metadata.authors.into_iter().map(AuthorDto::from).collect(),
                authors_lock: metadata.authors_lock,
                notable_entities: metadata
                    .notable_entities
                    .into_iter()
                    .map(NotableEntityDto::from)
                    .collect(),
                notable_entities_lock: metadata.notable_entities_lock,
                tags: metadata.tags,
                tags_lock: metadata.tags_lock,
                isbn: metadata.isbn,
                isbn_lock: metadata.isbn_lock,
                links: metadata.links.into_iter().map(WebLinkDto::from).collect(),
                links_lock: metadata.links_lock,
                created: JsonDateTime(metadata_created),
                last_modified: JsonDateTime(metadata_last_modified),
            },
            read_progress: read_progress.map(ReadProgressDto::from),
            deleted: book.deleted_date.is_some(),
            file_hash: book.file_hash,
            oneshot: book.oneshot,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaDto {
    status: &'static str,
    /// Empty before analysis.
    media_type: String,
    pages_count: i32,
    comment: String,
    epub_divina_compatible: bool,
    epub_is_kepub: bool,
    /// Empty for types without a profile.
    media_profile: &'static str,
}

impl From<MediaDetails> for MediaDto {
    fn from(media: MediaDetails) -> Self {
        let media_profile = match MediaProfile::from_media_type(media.media_type.as_deref()) {
            Some(MediaProfile::Divina) => "DIVINA",
            Some(MediaProfile::Epub) => "EPUB",
            Some(MediaProfile::Pdf) => "PDF",
            None => "",
        };
        Self {
            status: media.status.as_str(),
            media_type: media.media_type.unwrap_or_default(),
            pages_count: media.page_count,
            comment: media.comment.unwrap_or_default(),
            epub_divina_compatible: media.epub_divina_compatible,
            epub_is_kepub: media.epub_is_kepub,
            media_profile,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BookMetadataDto {
    title: String,
    title_lock: bool,
    summary: String,
    summary_lock: bool,
    number: String,
    number_lock: bool,
    number_sort: f32,
    number_sort_lock: bool,
    release_date: Option<Date>,
    release_date_lock: bool,
    authors: Vec<AuthorDto>,
    authors_lock: bool,
    notable_entities: Vec<NotableEntityDto>,
    notable_entities_lock: bool,
    tags: Vec<String>,
    tags_lock: bool,
    isbn: String,
    isbn_lock: bool,
    links: Vec<WebLinkDto>,
    links_lock: bool,
    created: JsonDateTime,
    last_modified: JsonDateTime,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthorDto {
    name: String,
    role: String,
}

impl From<BookAuthor> for AuthorDto {
    fn from(author: BookAuthor) -> Self {
        Self {
            name: author.name,
            role: author.role,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct NotableEntityDto {
    kind: String,
    name: String,
}

impl From<NotableEntity> for NotableEntityDto {
    fn from(entity: NotableEntity) -> Self {
        Self {
            kind: entity.kind,
            name: entity.name,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct WebLinkDto {
    label: String,
    url: String,
}

impl From<WebLink> for WebLinkDto {
    fn from(link: WebLink) -> Self {
        Self {
            label: link.label,
            url: link.url,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadProgressDto {
    page: i32,
    completed: bool,
    read_date: JsonDateTime,
    created: JsonDateTime,
    last_modified: JsonDateTime,
    device_id: String,
    device_name: String,
}

impl From<ReadProgressDetails> for ReadProgressDto {
    fn from(progress: ReadProgressDetails) -> Self {
        Self {
            page: progress.page,
            completed: progress.completed,
            read_date: JsonDateTime(progress.read_date),
            created: JsonDateTime(progress.created),
            last_modified: JsonDateTime(progress.last_modified),
            device_id: progress.device_id,
            device_name: progress.device_name,
        }
    }
}
