//! Kotlin `interfaces/api/kobo/dto` of library sync and reading states. Unlike the metadata,
//! these keep `null` properties unless Kotlin leaves them out.

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use komga_service::KoboReadingUpdate;
use komga_types::{
    BookId, KoboReadProgress, R2Location, R2Locator, SyncPointBook, SyncPointReadList,
};
use serde::{Deserialize, Serialize};

use super::dto::KoboBookMetadataDto;
use crate::mvc::jackson_date_time::{deserialize_instant, iso_offset_date_time};

fn utc(date: DateTime) -> String {
    let date = date
        .to_zoned_datetime(TimeZone::UTC)
        .expect("A stored date is within the supported range");
    iso_offset_date_time(&date)
}

/// One item of a sync page, tagged with its kind.
#[derive(Serialize)]
pub(super) enum SyncResultDto {
    NewEntitlement(BookEntitlementContainerDto),
    ChangedEntitlement(BookEntitlementContainerDto),
    ChangedProductMetadata(KoboBookMetadataDto),
    ChangedReadingState(WrappedReadingStateDto),
    NewTag(WrappedTagDto),
    ChangedTag(WrappedTagDto),
    DeletedTag(WrappedTagDto),
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct BookEntitlementContainerDto {
    book_entitlement: BookEntitlementDto,
    book_metadata: KoboBookMetadataDto,
    reading_state: Option<ReadingStateDto>,
}

impl BookEntitlementContainerDto {
    pub(super) fn new(
        book: &SyncPointBook,
        metadata: KoboBookMetadataDto,
        reading_state: ReadingStateDto,
    ) -> Self {
        Self {
            book_entitlement: BookEntitlementDto::new(book, false),
            book_metadata: metadata,
            reading_state: Some(reading_state),
        }
    }

    pub(super) fn removed(book: &SyncPointBook) -> Self {
        Self {
            book_entitlement: BookEntitlementDto::new(book, true),
            book_metadata: KoboBookMetadataDto::removed(&book.book_id),
            reading_state: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct BookEntitlementDto {
    accessibility: &'static str,
    active_period: PeriodDto,
    created: String,
    cross_revision_id: String,
    id: String,
    is_hidden_from_archive: bool,
    is_locked: bool,
    is_removed: bool,
    last_modified: String,
    origin_category: &'static str,
    revision_id: String,
    status: &'static str,
}

impl BookEntitlementDto {
    fn new(book: &SyncPointBook, is_removed: bool) -> Self {
        let id = book.book_id.to_string();
        Self {
            accessibility: "Full",
            active_period: PeriodDto {
                from: iso_offset_date_time(&Timestamp::now().to_zoned_datetime(TimeZone::UTC)),
            },
            created: utc(book.created_date),
            cross_revision_id: id.clone(),
            revision_id: id.clone(),
            id,
            is_hidden_from_archive: false,
            is_locked: false,
            is_removed,
            last_modified: utc(book.file_last_modified),
            origin_category: "Imported",
            status: "Active",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PeriodDto {
    from: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct WrappedReadingStateDto {
    pub reading_state: ReadingStateDto,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct ReadingStateDto {
    created: String,
    current_bookmark: BookmarkDto,
    entitlement_id: String,
    last_modified: String,
    priority_timestamp: String,
    statistics: StatisticsDto,
    status_info: StatusInfoDto,
}

impl ReadingStateDto {
    /// The user's progress, or a book ready to read since `book_created`.
    pub(super) fn new(
        id: &BookId,
        book_created: DateTime,
        progress: Option<&KoboReadProgress>,
    ) -> Self {
        let Some(progress) = progress else {
            let created = utc(book_created);
            return Self {
                created: created.clone(),
                current_bookmark: BookmarkDto {
                    last_modified: created.clone(),
                    progress_percent: None,
                    content_source_progress_percent: None,
                    location: None,
                },
                entitlement_id: id.to_string(),
                last_modified: created.clone(),
                priority_timestamp: created.clone(),
                statistics: StatisticsDto {
                    last_modified: created.clone(),
                },
                status_info: StatusInfoDto {
                    last_modified: created,
                    status: "ReadyToRead",
                    times_started_reading: 0,
                },
            };
        };
        let modified = utc(progress.last_modified);
        let locations = progress
            .locator
            .as_ref()
            .and_then(|locator| locator.locations.as_ref());
        Self {
            created: utc(progress.created),
            current_bookmark: BookmarkDto {
                last_modified: modified.clone(),
                progress_percent: locations
                    .and_then(|locations| locations.total_progression)
                    .map(|progression| progression * 100.0),
                content_source_progress_percent: locations
                    .and_then(|locations| locations.progression)
                    .map(|progression| progression * 100.0),
                location: progress.locator.as_ref().map(|locator| LocationDto {
                    value: locator.kobo_span.clone(),
                    kind: "KoboSpan",
                    source: locator.href.clone(),
                }),
            },
            entitlement_id: id.to_string(),
            last_modified: modified.clone(),
            priority_timestamp: modified.clone(),
            statistics: StatisticsDto {
                last_modified: modified.clone(),
            },
            status_info: StatusInfoDto {
                last_modified: modified,
                status: if progress.completed {
                    "Finished"
                } else {
                    "Reading"
                },
                times_started_reading: 1,
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct BookmarkDto {
    last_modified: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress_percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_source_progress_percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<LocationDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct LocationDto {
    value: Option<String>,
    #[serde(rename = "Type")]
    kind: &'static str,
    source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct StatisticsDto {
    last_modified: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct StatusInfoDto {
    last_modified: String,
    status: &'static str,
    times_started_reading: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct WrappedTagDto {
    tag: TagDto,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct TagDto {
    id: String,
    created: String,
    last_modified: String,
    name: String,
    #[serde(rename = "Type")]
    kind: &'static str,
    items: Option<Vec<TagItemDto>>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct TagItemDto {
    revision_id: String,
    #[serde(rename = "Type")]
    kind: &'static str,
}

impl WrappedTagDto {
    /// The tag's items are its books among `read_list_books`; a deleted tag has none.
    pub(super) fn new(
        read_list: &SyncPointReadList,
        read_list_books: Option<&[(String, BookId)]>,
    ) -> Self {
        Self {
            tag: TagDto {
                id: read_list.read_list_id.clone(),
                created: utc(read_list.created_date),
                last_modified: utc(read_list.last_modified_date),
                name: read_list.name.clone(),
                kind: "UserTag",
                items: read_list_books.map(|books| {
                    books
                        .iter()
                        .filter(|(id, _)| *id == read_list.read_list_id)
                        .map(|(_, book)| TagItemDto {
                            revision_id: book.to_string(),
                            kind: "ProductRevisionTagItem",
                        })
                        .collect()
                }),
            },
        }
    }
}

/// Kotlin `ReadingStateStateUpdateDto`, as far as the update needs it.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct ReadingStatesUpdateDto {
    #[serde(default)]
    reading_states: Vec<ReadingStateUpdateDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ReadingStateUpdateDto {
    #[serde(deserialize_with = "deserialize_instant")]
    last_modified: Timestamp,
    current_bookmark: BookmarkUpdateDto,
    status_info: StatusInfoUpdateDto,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BookmarkUpdateDto {
    progress_percent: Option<f32>,
    content_source_progress_percent: Option<f32>,
    location: Option<LocationUpdateDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LocationUpdateDto {
    value: Option<String>,
    #[serde(rename = "Type")]
    kind: Option<String>,
    source: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StatusInfoUpdateDto {
    status: String,
}

impl ReadingStatesUpdateDto {
    /// The first reading state; `None` without one or without a bookmark position.
    pub(super) fn into_update(self) -> Option<KoboReadingUpdate> {
        let state = self.reading_states.into_iter().next()?;
        let bookmark = state.current_bookmark;
        let location = bookmark.location?;
        let progression = bookmark.content_source_progress_percent?;
        if state.status_info.status == "Finished" {
            return Some(KoboReadingUpdate {
                modified: state.last_modified,
                locator: None,
            });
        }
        let is_kobo_span = location
            .kind
            .is_some_and(|kind| kind.eq_ignore_ascii_case("kobospan"));
        Some(KoboReadingUpdate {
            modified: state.last_modified,
            locator: Some(R2Locator {
                href: location.source,
                // Replaced by the resource's type when recorded.
                media_type: "application/xhtml+xml".to_owned(),
                kobo_span: location.value.filter(|_| is_kobo_span),
                locations: Some(R2Location {
                    progression: Some(progression / 100.0),
                    total_progression: bookmark.progress_percent.map(|percent| percent / 100.0),
                    ..R2Location::default()
                }),
                ..R2Locator::default()
            }),
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct RequestResultDto {
    request_result: &'static str,
    update_results: [ReadingStateUpdateResultDto; 1],
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ReadingStateUpdateResultDto {
    entitlement_id: String,
    current_bookmark_result: WrappedResultDto,
    statistics_result: WrappedResultDto,
    status_info_result: WrappedResultDto,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct WrappedResultDto {
    result: &'static str,
}

impl RequestResultDto {
    pub(super) fn new(id: &BookId, recorded: bool) -> Self {
        let (result, statistics) = if recorded {
            ("Success", "Ignored")
        } else {
            ("Failure", "Failure")
        };
        Self {
            request_result: result,
            update_results: [ReadingStateUpdateResultDto {
                entitlement_id: id.to_string(),
                current_bookmark_result: WrappedResultDto { result },
                statistics_result: WrappedResultDto { result: statistics },
                status_info_result: WrappedResultDto { result },
            }],
        }
    }
}
