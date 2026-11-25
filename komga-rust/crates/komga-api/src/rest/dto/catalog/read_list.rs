//! Read list requests and representation.

use komga_service::{NewReadList, ReadListUpdate};
use komga_types::{BookId, ReadList, ReadListId};
use serde::{Deserialize, Serialize};

use crate::error::Violation;
use crate::rest::validation;

/// `@JsonFormat(pattern = "yyyy-MM-dd'T'HH:mm:ss'Z'")`.
const DATE_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadListDto {
    pub id: ReadListId,
    pub name: String,
    pub summary: String,
    pub ordered: bool,
    pub book_ids: Vec<BookId>,
    pub created_date: String,
    pub last_modified_date: String,
    pub filtered: bool,
}

impl From<ReadList> for ReadListDto {
    fn from(read_list: ReadList) -> Self {
        Self {
            id: read_list.id,
            name: read_list.name,
            summary: read_list.summary,
            ordered: read_list.ordered,
            book_ids: read_list.book_ids,
            created_date: read_list
                .created_date
                .strftime(DATE_TIME_FORMAT)
                .to_string(),
            last_modified_date: read_list
                .last_modified_date
                .strftime(DATE_TIME_FORMAT)
                .to_string(),
            filtered: read_list.filtered,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadListCreationDto {
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default = "ordered_by_default")]
    pub ordered: bool,
    pub book_ids: Vec<BookId>,
}

fn ordered_by_default() -> bool {
    true
}

impl ReadListCreationDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::not_blank("name", &self.name, &mut violations);
        validation::not_empty("bookIds", &self.book_ids, &mut violations);
        validation::unique_elements("bookIds", Some(&self.book_ids), &mut violations);
        violations
    }

    pub(crate) fn into_new(self) -> NewReadList {
        NewReadList {
            name: self.name,
            summary: self.summary,
            ordered: self.ordered,
            book_ids: self.book_ids,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadListUpdateDto {
    pub name: Option<String>,
    pub summary: Option<String>,
    pub ordered: Option<bool>,
    pub book_ids: Option<Vec<BookId>>,
}

impl ReadListUpdateDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::null_or_not_blank("name", self.name.as_deref(), &mut violations);
        validation::null_or_not_empty("bookIds", self.book_ids.as_deref(), &mut violations);
        validation::unique_elements("bookIds", self.book_ids.as_deref(), &mut violations);
        violations
    }

    pub(crate) fn into_update(self) -> ReadListUpdate {
        ReadListUpdate {
            name: self.name,
            summary: self.summary,
            ordered: self.ordered,
            book_ids: self.book_ids,
        }
    }
}
