//! Kotlin `PageDto`.

use komga_types::BookPage;
use serde::Serialize;

use crate::mvc::media::format_size;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageDto {
    number: usize,
    file_name: String,
    media_type: String,
    width: Option<u32>,
    height: Option<u32>,
    size_bytes: Option<i64>,
    size: String,
}

impl PageDto {
    /// The page at `index` in page order, numbered from 1.
    #[must_use]
    pub fn new(index: usize, page: BookPage) -> Self {
        Self {
            number: index + 1,
            size: page.file_size.map(format_size).unwrap_or_default(),
            file_name: page.file_name,
            media_type: page.media_type,
            width: page.dimension.map(|dimension| dimension.width),
            height: page.dimension.map(|dimension| dimension.height),
            size_bytes: page.file_size,
        }
    }
}
