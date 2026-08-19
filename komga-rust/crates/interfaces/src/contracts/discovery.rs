use anyhow::{Context, Result};
use komga_application::discovery::{
    BookMetadataAuthorReadModel, BookMetadataLinkReadModel, BookReadModel,
    BookReadProgressReadModel,
};
use komga_domain::discovery::MediaProfile;
use serde::Serialize;

use super::common::{KotlinLocalDate, KotlinUtcDateTime};
use crate::helpers::{api_file_path, restricted_book_url};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDto {
    pub id: String,
    pub series_id: String,
    pub series_title: String,
    pub library_id: String,
    pub name: String,
    pub url: String,
    pub number: i32,
    pub created: KotlinUtcDateTime,
    pub last_modified: KotlinUtcDateTime,
    pub file_last_modified: KotlinUtcDateTime,
    pub size_bytes: u64,
    pub size: String,
    pub media: MediaDto,
    pub metadata: BookMetadataDto,
    pub read_progress: Option<ReadProgressDto>,
    pub deleted: bool,
    pub file_hash: String,
    pub oneshot: bool,
}

impl BookDto {
    pub fn from_read_model(book: &BookReadModel, is_admin: bool) -> Result<Self> {
        let url = restricted_book_url(&api_file_path(&book.url), is_admin);

        Ok(Self {
            id: book.id.clone(),
            series_id: book.series_id.clone(),
            series_title: book.series_title.clone(),
            library_id: book.library_id.clone(),
            name: book.name.clone(),
            url,
            number: book.number,
            created: parse_datetime("book.created", &book.created)?,
            last_modified: parse_datetime("book.lastModified", &book.last_modified)?,
            file_last_modified: parse_datetime("book.fileLastModified", &book.file_last_modified)?,
            size: format_size_bytes(book.size_bytes),
            size_bytes: book.size_bytes,
            media: MediaDto::from_read_model(book),
            metadata: BookMetadataDto::from_read_model(book)?,
            read_progress: book
                .read_progress
                .as_ref()
                .map(ReadProgressDto::from_read_model)
                .transpose()?,
            deleted: book.deleted,
            file_hash: book.file_hash.clone(),
            oneshot: book.oneshot,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDto {
    pub status: String,
    pub media_type: String,
    pub pages_count: u32,
    pub comment: String,
    pub epub_divina_compatible: bool,
    pub epub_is_kepub: bool,
    pub media_profile: String,
}

impl MediaDto {
    fn from_read_model(book: &BookReadModel) -> Self {
        Self {
            status: book.media_status.persisted_name().to_string(),
            media_type: book.media_type.clone(),
            pages_count: book.media_pages_count,
            comment: book.media_comment.clone(),
            epub_divina_compatible: book.media_epub_divina_compatible,
            epub_is_kepub: book.media_epub_is_kepub,
            media_profile: MediaProfile::from_media_type(&book.media_type)
                .map(MediaProfile::api_name)
                .unwrap_or_default()
                .to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadataDto {
    pub title: String,
    pub title_lock: bool,
    pub summary: String,
    pub summary_lock: bool,
    pub number: String,
    pub number_lock: bool,
    pub number_sort: f64,
    pub number_sort_lock: bool,
    pub release_date: Option<KotlinLocalDate>,
    pub release_date_lock: bool,
    pub authors: Vec<AuthorDto>,
    pub authors_lock: bool,
    pub tags: Vec<String>,
    pub tags_lock: bool,
    pub isbn: String,
    pub isbn_lock: bool,
    pub links: Vec<WebLinkDto>,
    pub links_lock: bool,
    pub created: KotlinUtcDateTime,
    pub last_modified: KotlinUtcDateTime,
}

impl BookMetadataDto {
    fn from_read_model(book: &BookReadModel) -> Result<Self> {
        Ok(Self {
            title: book.metadata_title.clone(),
            title_lock: book.metadata_title_lock,
            summary: book.metadata_summary.clone(),
            summary_lock: book.metadata_summary_lock,
            number: book.metadata_number.clone(),
            number_lock: book.metadata_number_lock,
            number_sort: book.metadata_number_sort,
            number_sort_lock: book.metadata_number_sort_lock,
            release_date: book
                .metadata_release_date
                .as_deref()
                .map(KotlinLocalDate::parse)
                .transpose()
                .context("book.metadata.releaseDate")?,
            release_date_lock: book.metadata_release_date_lock,
            authors: book
                .metadata_authors
                .iter()
                .map(AuthorDto::from_read_model)
                .collect(),
            authors_lock: book.metadata_authors_lock,
            tags: book.metadata_tags.clone(),
            tags_lock: book.metadata_tags_lock,
            isbn: book.metadata_isbn.clone(),
            isbn_lock: book.metadata_isbn_lock,
            links: book
                .metadata_links
                .iter()
                .map(WebLinkDto::from_read_model)
                .collect(),
            links_lock: book.metadata_links_lock,
            created: parse_datetime("book.metadata.created", &book.metadata_created)?,
            last_modified: parse_datetime(
                "book.metadata.lastModified",
                &book.metadata_last_modified,
            )?,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorDto {
    pub name: String,
    pub role: String,
}

impl AuthorDto {
    fn from_read_model(author: &BookMetadataAuthorReadModel) -> Self {
        Self {
            name: author.name.clone(),
            role: author.role.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebLinkDto {
    pub label: String,
    pub url: String,
}

impl WebLinkDto {
    fn from_read_model(link: &BookMetadataLinkReadModel) -> Self {
        Self {
            label: link.label.clone(),
            url: link.url.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressDto {
    pub page: i32,
    pub completed: bool,
    pub read_date: KotlinUtcDateTime,
    pub created: KotlinUtcDateTime,
    pub last_modified: KotlinUtcDateTime,
    pub device_id: String,
    pub device_name: String,
}

impl ReadProgressDto {
    fn from_read_model(progress: &BookReadProgressReadModel) -> Result<Self> {
        let read_date = progress
            .read_date
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                (!progress.last_modified.trim().is_empty())
                    .then_some(progress.last_modified.as_str())
            })
            .unwrap_or(progress.created.as_str());

        Ok(Self {
            page: progress.page,
            completed: progress.completed,
            read_date: parse_datetime("book.readProgress.readDate", read_date)?,
            created: parse_datetime("book.readProgress.created", &progress.created)?,
            last_modified: parse_datetime(
                "book.readProgress.lastModified",
                &progress.last_modified,
            )?,
            device_id: progress.device_id.clone(),
            device_name: progress.device_name.clone(),
        })
    }
}

fn parse_datetime(field: &str, raw: &str) -> Result<KotlinUtcDateTime> {
    KotlinUtcDateTime::parse(raw).with_context(|| format!("invalid {field}: {raw}"))
}

pub fn format_size_bytes(size_bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    if size_bytes < 1024 {
        return format!("{size_bytes} B");
    }

    let mut size = size_bytes as f64;
    let mut unit_index = 0usize;
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if (size - size.round()).abs() < 0.05 {
        format!("{} {}", size.round() as u64, UNITS[unit_index])
    } else {
        format!("{size:.1} {}", UNITS[unit_index])
    }
}
