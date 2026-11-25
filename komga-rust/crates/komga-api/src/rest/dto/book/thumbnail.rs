//! Kotlin `ThumbnailBookDto`, `ThumbnailSeriesDto`, `ThumbnailReadListDto` and
//! `ThumbnailSeriesCollectionDto`.

use komga_types::{BookId, CollectionId, ReadListId, SeriesId, ThumbnailDetails, ThumbnailId};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailBookDto {
    id: ThumbnailId,
    book_id: BookId,
    #[serde(rename = "type")]
    kind: &'static str,
    selected: bool,
    media_type: String,
    file_size: i64,
    width: u32,
    height: u32,
}

impl ThumbnailBookDto {
    #[must_use]
    pub fn new(book_id: &BookId, thumbnail: ThumbnailDetails) -> Self {
        Self {
            id: thumbnail.id,
            book_id: book_id.clone(),
            kind: thumbnail.kind.name(),
            selected: thumbnail.selected,
            media_type: thumbnail.media_type,
            file_size: thumbnail.file_size,
            width: thumbnail.dimension.width,
            height: thumbnail.dimension.height,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSeriesDto {
    id: ThumbnailId,
    series_id: SeriesId,
    #[serde(rename = "type")]
    kind: &'static str,
    selected: bool,
    media_type: String,
    file_size: i64,
    width: u32,
    height: u32,
}

impl ThumbnailSeriesDto {
    #[must_use]
    pub fn new(series_id: &SeriesId, thumbnail: ThumbnailDetails) -> Self {
        Self {
            id: thumbnail.id,
            series_id: series_id.clone(),
            kind: thumbnail.kind.name(),
            selected: thumbnail.selected,
            media_type: thumbnail.media_type,
            file_size: thumbnail.file_size,
            width: thumbnail.dimension.width,
            height: thumbnail.dimension.height,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailReadListDto {
    id: ThumbnailId,
    read_list_id: ReadListId,
    #[serde(rename = "type")]
    kind: &'static str,
    selected: bool,
    media_type: String,
    file_size: i64,
    width: u32,
    height: u32,
}

impl ThumbnailReadListDto {
    #[must_use]
    pub fn new(read_list_id: &ReadListId, thumbnail: ThumbnailDetails) -> Self {
        Self {
            id: thumbnail.id,
            read_list_id: read_list_id.clone(),
            kind: thumbnail.kind.name(),
            selected: thumbnail.selected,
            media_type: thumbnail.media_type,
            file_size: thumbnail.file_size,
            width: thumbnail.dimension.width,
            height: thumbnail.dimension.height,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSeriesCollectionDto {
    id: ThumbnailId,
    collection_id: CollectionId,
    #[serde(rename = "type")]
    kind: &'static str,
    selected: bool,
    media_type: String,
    file_size: i64,
    width: u32,
    height: u32,
}

impl ThumbnailSeriesCollectionDto {
    #[must_use]
    pub fn new(collection_id: &CollectionId, thumbnail: ThumbnailDetails) -> Self {
        Self {
            id: thumbnail.id,
            collection_id: collection_id.clone(),
            kind: thumbnail.kind.name(),
            selected: thumbnail.selected,
            media_type: thumbnail.media_type,
            file_size: thumbnail.file_size,
            width: thumbnail.dimension.width,
            height: thumbnail.dimension.height,
        }
    }
}
