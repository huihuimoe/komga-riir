use komga_types::{CollectionId, CollectionRef, SeriesId};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct CollectionSseDto<'a> {
    pub collection_id: &'a CollectionId,
    pub series_ids: &'a [SeriesId],
}

impl<'a> From<&'a CollectionRef> for CollectionSseDto<'a> {
    fn from(collection: &'a CollectionRef) -> Self {
        Self {
            collection_id: &collection.collection_id,
            series_ids: &collection.series_ids,
        }
    }
}
