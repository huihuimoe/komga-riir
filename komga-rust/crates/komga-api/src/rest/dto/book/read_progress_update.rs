//! `ReadProgressUpdateDto` of `BookController.markBookReadProgress`.

use serde::Deserialize;

use crate::error::ApiError;
use crate::rest::validation;

#[derive(Debug, Deserialize)]
pub struct ReadProgressUpdateDto {
    page: Option<i32>,
    completed: Option<bool>,
}

/// What a valid update marks.
pub enum ReadProgressMark {
    Completed,
    Page(i32),
}

impl ReadProgressUpdateDto {
    /// `@Positive page` and `@ReadProgressUpdateDtoConstraint` (a page unless completed). Only
    /// field violations are listed, so a missing page is a 400 without violations.
    pub(crate) fn validate(self) -> Result<ReadProgressMark, ApiError> {
        let mut violations = Vec::new();
        if let Some(page) = self.page {
            validation::positive("page", page.into(), &mut violations);
        }
        let mark = if self.completed == Some(true) {
            Some(ReadProgressMark::Completed)
        } else {
            self.page.map(ReadProgressMark::Page)
        };
        match mark {
            Some(mark) if violations.is_empty() => Ok(mark),
            _ => Err(ApiError::Validation(violations)),
        }
    }
}
