use komga_types::{LibraryId, SeriesId, SeriesRef};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct SeriesSseDto<'a> {
    pub series_id: &'a SeriesId,
    pub library_id: &'a LibraryId,
}

impl<'a> From<&'a SeriesRef> for SeriesSseDto<'a> {
    fn from(series: &'a SeriesRef) -> Self {
        Self {
            series_id: &series.series_id,
            library_id: &series.library_id,
        }
    }
}
