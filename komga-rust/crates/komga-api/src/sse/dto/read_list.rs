use komga_types::{BookId, ReadListId, ReadListRef};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct ReadListSseDto<'a> {
    pub read_list_id: &'a ReadListId,
    pub book_ids: &'a [BookId],
}

impl<'a> From<&'a ReadListRef> for ReadListSseDto<'a> {
    fn from(list: &'a ReadListRef) -> Self {
        Self {
            read_list_id: &list.read_list_id,
            book_ids: &list.book_ids,
        }
    }
}
