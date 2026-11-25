//! Library creation and role-sensitive representation.

use std::path::PathBuf;

use komga_types::{Library, LibraryId};
use serde::{Deserialize, Serialize};

use super::library_options::LibraryOptionsDto;
use crate::error::Violation;
use crate::rest::validation;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCreationDto {
    pub name: String,
    pub root: String,
    #[serde(flatten)]
    pub options: LibraryOptionsDto,
}

impl LibraryCreationDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::not_blank("name", &self.name, &mut violations);
        validation::not_blank("root", &self.root, &mut violations);
        violations
    }

    pub(crate) fn into_library(self) -> Library {
        let mut library = Library::new(self.name, PathBuf::from(self.root));
        self.options.apply(&mut library);
        library
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDto {
    pub id: LibraryId,
    pub name: String,
    pub root: String,
    #[serde(flatten)]
    pub options: LibraryOptionsDto,
    pub unavailable: bool,
}

impl LibraryDto {
    pub(crate) fn from_library(mut library: Library, include_root: bool) -> Self {
        Self {
            id: library.id.clone(),
            name: std::mem::take(&mut library.name),
            root: if include_root {
                library.root.to_string_lossy().into_owned()
            } else {
                String::new()
            },
            unavailable: library.unavailable_date.is_some(),
            options: library.into(),
        }
    }
}
