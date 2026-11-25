use komga_types::{BookId, BookProgressRef, SeriesId, SeriesProgressRef, UserId};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct ReadProgressSseDto<'a> {
    pub book_id: &'a BookId,
    pub user_id: &'a UserId,
}

impl<'a> From<&'a BookProgressRef> for ReadProgressSseDto<'a> {
    fn from(progress: &'a BookProgressRef) -> Self {
        Self {
            book_id: &progress.book_id,
            user_id: &progress.user_id,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct ReadProgressSeriesSseDto<'a> {
    pub series_id: &'a SeriesId,
    pub user_id: &'a UserId,
}

impl<'a> From<&'a SeriesProgressRef> for ReadProgressSeriesSseDto<'a> {
    fn from(progress: &'a SeriesProgressRef) -> Self {
        Self {
            series_id: &progress.series_id,
            user_id: &progress.user_id,
        }
    }
}
