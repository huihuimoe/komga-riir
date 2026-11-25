use komga_types::{BookId, BookRef, CollectionId, ReadListId, SeriesId, SeriesRef, ThumbnailRef};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct ThumbnailBookSseDto<'a> {
    pub book_id: &'a BookId,
    pub series_id: &'a SeriesId,
    pub selected: bool,
}

impl<'a> From<&'a ThumbnailRef<BookRef>> for ThumbnailBookSseDto<'a> {
    fn from(thumbnail: &'a ThumbnailRef<BookRef>) -> Self {
        Self {
            book_id: &thumbnail.owner.book_id,
            series_id: &thumbnail.owner.series_id,
            selected: thumbnail.selected,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct ThumbnailSeriesSseDto<'a> {
    pub series_id: &'a SeriesId,
    pub selected: bool,
}

impl<'a> From<&'a ThumbnailRef<SeriesRef>> for ThumbnailSeriesSseDto<'a> {
    fn from(thumbnail: &'a ThumbnailRef<SeriesRef>) -> Self {
        Self {
            series_id: &thumbnail.owner.series_id,
            selected: thumbnail.selected,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct ThumbnailReadListSseDto<'a> {
    pub read_list_id: &'a ReadListId,
    pub selected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct ThumbnailSeriesCollectionSseDto<'a> {
    pub collection_id: &'a CollectionId,
    pub selected: bool,
}
