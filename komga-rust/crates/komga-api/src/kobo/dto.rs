//! Kotlin `interfaces/api/kobo/dto`: Kobo's `UpperCamelCase` JSON, `null` properties omitted.

use jiff::tz::TimeZone;
use komga_types::{BookId, KoboBook};
use serde::Serialize;

use crate::mvc::jackson_date_time::iso_offset_date_time;

/// Kotlin `DUMMY_ID`, the category and genre of every book.
const DUMMY_ID: &str = "00000000-0000-0000-0000-000000000001";

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct AuthDto {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub tracking_id: String,
    pub user_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct KoboBookMetadataDto {
    categories: [&'static str; 1],
    contributor_roles: Vec<ContributorDto>,
    contributors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover_image_id: Option<String>,
    cross_revision_id: String,
    current_display_price: AmountDto,
    current_love_display_price: AmountDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    download_urls: Vec<DownloadUrlDto>,
    entitlement_id: String,
    external_ids: [String; 0],
    genre: &'static str,
    is_eligible_for_kobo_love: bool,
    is_internet_archive: bool,
    is_pre_order: bool,
    is_social_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    isbn: Option<String>,
    language: String,
    phonetic_pronunciations: Empty,
    #[serde(skip_serializing_if = "Option::is_none")]
    publication_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    publisher: Option<PublisherDto>,
    revision_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    series: Option<KoboSeriesDto>,
    title: String,
    work_id: String,
}

#[derive(Serialize)]
struct Empty {}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ContributorDto {
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct AmountDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    currency_code: Option<&'static str>,
    total_amount: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct DownloadUrlDto {
    drm_type: &'static str,
    format: &'static str,
    size: i64,
    platform: &'static str,
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PublisherDto {
    imprint: &'static str,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct KoboSeriesDto {
    id: String,
    name: String,
    number: String,
    number_float: f32,
}

impl KoboBookMetadataDto {
    /// `download_url` is the book file URL without its `convert_kepub` query.
    pub(super) fn new(book: KoboBook, download_url: &str) -> Self {
        let id = book.id.to_string();
        // A fixed layout is delivered as is; anything else is converted to Kepub unless it is one.
        let (format, convert) = if book.is_fixed_layout {
            ("EPUB3FL", false)
        } else {
            ("KEPUB", !book.is_kepub)
        };
        let publication_date = match book.release_date {
            Some(date) => date.to_zoned_datetime(TimeZone::UTC),
            None => book.metadata_created.to_zoned_datetime(TimeZone::UTC),
        }
        .expect("A stored date is within the supported range");
        let language: String = book.language.chars().take(2).collect();
        Self {
            categories: [DUMMY_ID],
            contributor_roles: book
                .authors
                .iter()
                .map(|name| ContributorDto { name: name.clone() })
                .collect(),
            contributors: book.authors,
            cover_image_id: book.cover.map(|cover| cover.to_string()),
            cross_revision_id: id.clone(),
            current_display_price: AmountDto {
                currency_code: Some("USD"),
                total_amount: 0,
            },
            current_love_display_price: AmountDto {
                currency_code: None,
                total_amount: 0,
            },
            // Kobo keeps the previous description when this one is empty.
            description: Some(if book.summary.is_empty() {
                " ".to_owned()
            } else {
                book.summary
            }),
            download_urls: vec![DownloadUrlDto {
                drm_type: "None",
                format,
                size: book.file_size,
                platform: "Generic",
                url: format!("{download_url}?convert_kepub={convert}"),
            }],
            entitlement_id: id.clone(),
            external_ids: [],
            genre: DUMMY_ID,
            is_eligible_for_kobo_love: false,
            is_internet_archive: false,
            is_pre_order: false,
            is_social_enabled: true,
            isbn: Some(book.isbn).filter(|isbn| !isbn.trim().is_empty()),
            language: if language.trim().is_empty() {
                "en".to_owned()
            } else {
                language
            },
            phonetic_pronunciations: Empty {},
            publication_date: Some(iso_offset_date_time(&publication_date)),
            publisher: Some(PublisherDto {
                imprint: "",
                name: book.publisher,
            }),
            revision_id: id.clone(),
            series: (!book.oneshot).then(|| KoboSeriesDto {
                id: book.series_id.to_string(),
                name: book.series_title,
                number: book.number,
                number_float: book.number_sort,
            }),
            title: book.title,
            work_id: id,
        }
    }

    /// What Kobo is told of a book it should drop: its id in place of everything.
    pub(super) fn removed(id: &BookId) -> Self {
        let id = id.to_string();
        Self {
            categories: [DUMMY_ID],
            contributor_roles: Vec::new(),
            contributors: Vec::new(),
            cover_image_id: Some(id.clone()),
            cross_revision_id: id.clone(),
            current_display_price: AmountDto {
                currency_code: Some("USD"),
                total_amount: 0,
            },
            current_love_display_price: AmountDto {
                currency_code: None,
                total_amount: 0,
            },
            description: None,
            download_urls: Vec::new(),
            entitlement_id: id.clone(),
            external_ids: [],
            genre: DUMMY_ID,
            is_eligible_for_kobo_love: false,
            is_internet_archive: false,
            is_pre_order: false,
            is_social_enabled: true,
            isbn: None,
            language: "en".to_owned(),
            phonetic_pronunciations: Empty {},
            publication_date: None,
            publisher: None,
            revision_id: id.clone(),
            series: None,
            title: id.clone(),
            work_id: id,
        }
    }
}
