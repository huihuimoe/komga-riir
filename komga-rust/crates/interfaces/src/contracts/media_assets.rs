use komga_application::media_assets::{
    CollectionThumbnailRecord, EntityThumbnailRecord, ReadlistThumbnailRecord,
    SeriesThumbnailRecord,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailFieldsDto {
    #[serde(rename = "type")]
    pub thumbnail_type: String,
    pub selected: bool,
    pub media_type: String,
    pub file_size: i64,
    pub width: i64,
    pub height: i64,
}

impl ThumbnailFieldsDto {
    fn new(
        thumbnail_type: &komga_domain::media_assets::ThumbnailType,
        selected: bool,
        media_type: &str,
        file_size: i64,
        width: i64,
        height: i64,
    ) -> Self {
        Self {
            thumbnail_type: thumbnail_type.persisted_name().to_string(),
            selected,
            media_type: media_type.to_string(),
            file_size,
            width,
            height,
        }
    }

    pub fn from_book_record(record: &EntityThumbnailRecord) -> Self {
        Self::new(
            &record.thumbnail_type,
            record.selected,
            &record.media_type,
            record.file_size,
            record.width,
            record.height,
        )
    }

    pub fn from_series_record(record: &SeriesThumbnailRecord) -> Self {
        Self::new(
            &record.thumbnail_type,
            record.selected,
            &record.media_type,
            record.file_size,
            record.width,
            record.height,
        )
    }

    pub fn from_readlist_record(record: &ReadlistThumbnailRecord) -> Self {
        Self::new(
            &record.thumbnail_type,
            record.selected,
            &record.media_type,
            record.file_size,
            record.width,
            record.height,
        )
    }

    pub fn from_collection_record(record: &CollectionThumbnailRecord) -> Self {
        Self::new(
            &record.thumbnail_type,
            record.selected,
            &record.media_type,
            record.file_size,
            record.width,
            record.height,
        )
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookThumbnailDto {
    pub id: String,
    pub book_id: String,
    #[serde(flatten)]
    pub fields: ThumbnailFieldsDto,
}

impl BookThumbnailDto {
    pub fn from_record(record: &EntityThumbnailRecord) -> Self {
        Self {
            id: record.id.clone(),
            book_id: record.book_id.clone(),
            fields: ThumbnailFieldsDto::from_book_record(record),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesThumbnailDto {
    pub id: String,
    pub series_id: String,
    #[serde(flatten)]
    pub fields: ThumbnailFieldsDto,
}

impl SeriesThumbnailDto {
    pub fn from_record(record: &SeriesThumbnailRecord) -> Self {
        Self {
            id: record.id.clone(),
            series_id: record.series_id.clone(),
            fields: ThumbnailFieldsDto::from_series_record(record),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadListThumbnailDto {
    pub id: String,
    #[serde(rename = "readListId")]
    pub readlist_id: String,
    #[serde(flatten)]
    pub fields: ThumbnailFieldsDto,
}

impl ReadListThumbnailDto {
    pub fn from_record(record: &ReadlistThumbnailRecord) -> Self {
        Self {
            id: record.id.clone(),
            readlist_id: record.readlist_id.clone(),
            fields: ThumbnailFieldsDto::from_readlist_record(record),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionThumbnailDto {
    pub id: String,
    pub collection_id: String,
    #[serde(flatten)]
    pub fields: ThumbnailFieldsDto,
}

impl CollectionThumbnailDto {
    pub fn from_record(record: &CollectionThumbnailRecord) -> Self {
        Self {
            id: record.id.clone(),
            collection_id: record.collection_id.clone(),
            fields: ThumbnailFieldsDto::from_collection_record(record),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReadiumPositionListDto {
    pub total: usize,
    pub positions: Vec<serde_json::Value>,
}
