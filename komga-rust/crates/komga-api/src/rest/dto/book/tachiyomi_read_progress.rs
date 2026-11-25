//! Kotlin `TachiyomiReadProgressDto`, `TachiyomiReadProgressV2Dto`, and their update DTOs of the
//! Mihon endpoints.

use komga_service::{ReadListTachiyomiProgress, SeriesTachiyomiProgress};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::rest::validation;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TachiyomiReadProgressDto {
    books_count: i32,
    books_read_count: i32,
    books_unread_count: i32,
    books_in_progress_count: i32,
    last_read_continuous_index: i32,
}

impl From<ReadListTachiyomiProgress> for TachiyomiReadProgressDto {
    fn from(progress: ReadListTachiyomiProgress) -> Self {
        let counts = progress.counts;
        Self {
            books_count: counts.total(),
            books_read_count: counts.read,
            books_unread_count: counts.unread,
            books_in_progress_count: counts.in_progress,
            last_read_continuous_index: progress.last_read_continuous_index,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TachiyomiReadProgressUpdateDto {
    last_book_read: i32,
}

impl TachiyomiReadProgressUpdateDto {
    /// `@PositiveOrZero lastBookRead`: how many leading books to complete.
    pub(crate) fn validate(self) -> Result<usize, ApiError> {
        let mut violations = Vec::new();
        validation::positive_or_zero("lastBookRead", self.last_book_read, &mut violations);
        usize::try_from(self.last_book_read).map_err(|_| ApiError::Validation(violations))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TachiyomiReadProgressV2Dto {
    books_count: i32,
    books_read_count: i32,
    books_unread_count: i32,
    books_in_progress_count: i32,
    last_read_continuous_number_sort: f32,
    max_number_sort: f32,
}

impl From<SeriesTachiyomiProgress> for TachiyomiReadProgressV2Dto {
    fn from(progress: SeriesTachiyomiProgress) -> Self {
        let counts = progress.counts;
        Self {
            books_count: counts.total(),
            books_read_count: counts.read,
            books_unread_count: counts.unread,
            books_in_progress_count: counts.in_progress,
            last_read_continuous_number_sort: progress.last_read_continuous_number_sort,
            max_number_sort: progress.max_number_sort,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TachiyomiReadProgressUpdateV2Dto {
    pub last_book_number_sort_read: f32,
}
