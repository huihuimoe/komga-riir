//! Kotlin `BookSearch`: the body of `POST /api/v1/books/list`.

use komga_types::BookCondition;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSearchDto {
    pub condition: Option<BookCondition>,
    pub full_text_search: Option<String>,
}
