//! Komga `PageResponse`, the page representation of the v3 API.

use komga_types::{PageRequest, PageResult};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageResponse<T> {
    content: Vec<T>,
    page: u32,
    size: u64,
    total_elements: u64,
    total_pages: u64,
    first: bool,
    last: bool,
}

impl<T> PageResponse<T> {
    /// `PageResult.toPageResponse`: an unpaged result is page 0 sized by its items.
    pub(crate) fn new<S: Into<T>>(result: PageResult<S>, request: PageRequest) -> Self {
        let content: Vec<T> = result.items.into_iter().map(Into::into).collect();
        let total_elements = result.total_elements;
        let (page, size, total_pages) = match request {
            PageRequest::Paged { number, size } => (
                number,
                u64::from(size),
                total_elements.div_ceil(u64::from(size)),
            ),
            PageRequest::Unpaged => (0, content.len() as u64, u64::from(total_elements > 0)),
        };
        Self {
            content,
            page,
            size,
            total_elements,
            total_pages,
            first: page == 0,
            last: u64::from(page) + 1 >= total_pages,
        }
    }
}
