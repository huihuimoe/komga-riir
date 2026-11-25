use komga_types::LibraryId;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct LibrarySseDto<'a> {
    pub library_id: &'a LibraryId,
}

impl<'a> From<&'a LibraryId> for LibrarySseDto<'a> {
    fn from(library_id: &'a LibraryId) -> Self {
        Self { library_id }
    }
}
