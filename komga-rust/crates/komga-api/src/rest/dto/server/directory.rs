//! Host filesystem browser request and response representations.

use komga_service::{DirectoryEntry, DirectoryListing};
use serde::de::value::MapAccessDeserializer;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Default, Deserialize)]
#[serde(remote = "Self", rename_all = "camelCase", default)]
pub struct DirectoryRequestDto {
    pub path: String,
    pub show_files: bool,
}

impl<'de> Deserialize<'de> for DirectoryRequestDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ObjectVisitor;

        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = DirectoryRequestDto;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a directory request object")
            }

            fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                DirectoryRequestDto::deserialize(MapAccessDeserializer::new(map))
            }
        }

        // Unlike Jackson, a derived serde struct with defaults also accepts an empty array.
        deserializer.deserialize_map(ObjectVisitor)
    }
}

#[derive(Serialize)]
pub struct DirectoryListingDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub directories: Vec<PathDto>,
    pub files: Vec<PathDto>,
}

#[derive(Serialize)]
pub struct PathDto {
    #[serde(rename = "type")]
    pub entry_type: &'static str,
    pub name: String,
    pub path: String,
}

impl From<DirectoryListing> for DirectoryListingDto {
    fn from(listing: DirectoryListing) -> Self {
        Self {
            parent: listing
                .parent
                .map(|path| path.to_string_lossy().into_owned()),
            directories: listing
                .directories
                .into_iter()
                .map(|entry| path_dto(entry, "directory"))
                .collect(),
            files: listing
                .files
                .into_iter()
                .map(|entry| path_dto(entry, "file"))
                .collect(),
        }
    }
}

fn path_dto(entry: DirectoryEntry, entry_type: &'static str) -> PathDto {
    PathDto {
        entry_type,
        name: entry.name,
        path: entry.path.to_string_lossy().into_owned(),
    }
}
