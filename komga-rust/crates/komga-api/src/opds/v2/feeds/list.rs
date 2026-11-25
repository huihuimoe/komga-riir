//! Collection and read list OPDS 2 feeds.

use axum::extract::{Path, State};
use axum::response::Response;
use komga_types::{CollectionId, LibraryId, ReadListId};

use super::{group, library_metadata, library_navigation, library_root, paged_feed};
use crate::error::ApiError;
use crate::opds::dto::feed::FeedMetadata;
use crate::opds::v2::request::{Opds2, respond};
use crate::opds::{OpdsState, server_time};

/// Which lists a library's paged list feed shows.
#[derive(Clone, Copy)]
pub enum ListFeed {
    Collections,
    ReadLists,
}

pub async fn lists(
    kind: ListFeed,
    State(state): State<OpdsState>,
    id: Option<Path<LibraryId>>,
    request: Opds2,
) -> Result<Response, ApiError> {
    let id = id.map(|Path(id)| id);
    let library = request.library(&state, id.as_ref()).await?;
    let query = request.query(&state);
    let (title, path, links, total) = match kind {
        ListFeed::Collections => {
            let lists = query.collections(id.as_ref(), None).await?;
            let links: Vec<_> = lists
                .items
                .into_iter()
                .map(|it| request.navigation(it.name, &format!("collections/{}", it.id)))
                .collect();
            ("Collections", "collections", links, lists.total_elements)
        }
        ListFeed::ReadLists => {
            let lists = query.read_lists(id.as_ref(), None).await?;
            let links: Vec<_> = lists
                .items
                .into_iter()
                .map(|it| request.navigation(it.name, &format!("readlists/{}", it.id)))
                .collect();
            ("Read Lists", "readlists", links, lists.total_elements)
        }
    };
    let href = request.href(&format!("{}/{path}", library_root(id.as_ref())));
    let metadata = library_metadata(library.as_ref(), &format!(" - {title}"));
    let mut feed = paged_feed(&request, metadata, href, links.len(), total);
    feed.navigation = library_navigation(&state, &request, id.as_ref()).await?;
    feed.groups.push(group(title, links, Vec::new()));
    Ok(respond(&feed))
}

pub async fn collection(
    State(state): State<OpdsState>,
    Path(id): Path<CollectionId>,
    request: Opds2,
) -> Result<Response, ApiError> {
    let collection = state
        .collections
        .get(&request.user, &id)
        .await
        .map_err(|error| ApiError::from_collection(&error, &request.path))?;
    let series = request.query(&state).collection_series(&id).await?;
    let href = request.href(&format!("collections/{id}"));
    let metadata = FeedMetadata {
        title: collection.name,
        modified: Some(server_time(collection.last_modified_date)),
        description: None,
        page: None,
    };
    let mut feed = paged_feed(
        &request,
        metadata,
        href,
        series.items.len(),
        series.total_elements,
    );
    feed.navigation = request.series(series.items);
    Ok(respond(&feed))
}

pub async fn read_list(
    State(state): State<OpdsState>,
    Path(id): Path<ReadListId>,
    request: Opds2,
) -> Result<Response, ApiError> {
    let read_list = state
        .read_lists
        .get(&request.user, &id)
        .await
        .map_err(|error| ApiError::from_read_list(&error, &request.path))?;
    let books = request.query(&state).read_list_books(&id).await?;
    let href = request.href(&format!("readlists/{id}"));
    let metadata = FeedMetadata {
        title: read_list.name,
        modified: Some(server_time(read_list.last_modified_date)),
        description: None,
        page: None,
    };
    let mut feed = paged_feed(
        &request,
        metadata,
        href,
        books.items.len(),
        books.total_elements,
    );
    feed.publications = request.publications(&books.items);
    Ok(respond(&feed))
}
