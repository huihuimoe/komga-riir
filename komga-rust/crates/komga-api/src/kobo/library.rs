//! Kotlin `KoboController.syncLibrary`, `getState` and `updateState`.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{AppendHeaders, IntoResponse, Response};
use axum::{Extension, Json};
use komga_service::{KoboChanges, KoboReply, KoboSync};
use komga_types::{ApiKey, BookId, KoboBook, R2Device, SyncPointBook, SyncPointReadList};
use serde_json::Value;
use tracing::error;

use super::dto::KoboBookMetadataDto;
use super::sync_dto::{
    BookEntitlementContainerDto, ReadingStateDto, ReadingStatesUpdateDto, RequestResultDto,
    SyncResultDto, WrappedReadingStateDto, WrappedTagDto,
};
use super::sync_token::SyncToken;
use super::{KoboState, SYNC_TOKEN, device_url, from_kobo, settings, to_store};
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::servlet_uri::{RequestContext, encode_path};

pub(super) async fn sync(
    State(state): State<KoboState>,
    Authenticated(user): Authenticated,
    Extension(api_key): Extension<ApiKey>,
    Path(key): Path<String>,
    headers: HeaderMap,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let mut token = headers
        .get(SYNC_TOKEN)
        .and_then(|header| header.to_str().ok())
        .map(SyncToken::parse)
        .unwrap_or_default();
    let KoboSync {
        round,
        changes,
        mut more,
    } = state
        .kobo
        .sync(&user, Some(&api_key.id), &token.anchors())
        .await
        .map_err(|error| from_kobo(&error, uri.path()))?;
    let settings = settings(&state, &uri).await?;
    let books_url = format!(
        "{}/kobo/{}/v1/books",
        device_url(&state, settings, &context),
        encode_path(&key)
    );
    let mut items: Vec<Value> = sync_items(changes, &books_url)
        .into_iter()
        .map(|item| serde_json::to_value(item).expect("Sync items serialize"))
        .collect();
    // Kotlin asks the store only once Komga's own changes are all handed out.
    if !more && settings.proxy {
        more = merge_store_sync(&state, &uri, &headers, &mut token, &mut items).await;
    }
    let anchors = state
        .kobo
        .finish_sync(round, more)
        .await
        .map_err(|error| from_kobo(&error, uri.path()))?;
    let token = token.with_anchors(anchors).encode();
    let more = more.then_some(("x-kobo-sync", "continue"));
    Ok(([(SYNC_TOKEN, token)], AppendHeaders(more), Json(items)).into_response())
}

/// Appends the store's sync items and keeps its sync token; whether the store has more.
/// A failing store leaves the page as Komga made it.
async fn merge_store_sync(
    state: &KoboState,
    uri: &Uri,
    headers: &HeaderMap,
    token: &mut SyncToken,
    items: &mut Vec<Value>,
) -> bool {
    let store_token = Some(token.raw_kobo_sync_token.as_str());
    let reply = match state
        .store
        .forward(Method::GET, uri, headers, Bytes::new(), store_token)
        .await
    {
        Ok(reply) => reply,
        Err(error) => {
            error!(%error, "Kobo sync endpoint failure");
            return false;
        }
    };
    if let Some(Value::Array(store_items)) = reply.body {
        items.extend(store_items);
    }
    if let Some(store_token) = reply
        .headers
        .get(SYNC_TOKEN)
        .and_then(|value| value.to_str().ok())
    {
        store_token.clone_into(&mut token.raw_kobo_sync_token);
    }
    reply
        .headers
        .get("x-kobo-sync")
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"continue"))
}

/// The page's items in Kotlin's order: books, their metadata and reading states, then tags.
fn sync_items(changes: KoboChanges, books_url: &str) -> Vec<SyncResultDto> {
    let metadata: HashMap<_, _> = changes
        .metadata
        .into_iter()
        .map(|book| (book.id.clone(), book))
        .collect();
    let progress: HashMap<_, _> = changes
        .progress
        .into_iter()
        .map(|progress| (progress.book_id.clone(), progress))
        .collect();
    let metadata_dto = |book: &KoboBook| {
        let url = format!("{books_url}/{}/file/epub", book.id);
        KoboBookMetadataDto::new(book.clone(), &url)
    };
    // A book deleted since the sync point was taken has no metadata left to hand out.
    let entitled = |book: &SyncPointBook| {
        let metadata = metadata.get(&book.book_id)?;
        let state = ReadingStateDto::new(
            &book.book_id,
            book.created_date,
            progress.get(&book.book_id),
        );
        Some(SyncResultDto::NewEntitlement(
            BookEntitlementContainerDto::new(book, metadata_dto(metadata), state),
        ))
    };
    let mut items: Vec<_> = changes
        .books_added
        .iter()
        .chain(&changes.books_changed)
        .filter_map(entitled)
        .collect();
    items.extend(changes.books_changed.iter().filter_map(|book| {
        let metadata = metadata.get(&book.book_id)?;
        Some(SyncResultDto::ChangedProductMetadata(metadata_dto(
            metadata,
        )))
    }));
    items.extend(
        changes.books_removed.iter().map(|book| {
            SyncResultDto::ChangedEntitlement(BookEntitlementContainerDto::removed(book))
        }),
    );
    items.extend(
        changes
            .books_changed
            .iter()
            .chain(&changes.books_progress_changed)
            .filter_map(|book| {
                let progress = progress.get(&book.book_id)?;
                let reading_state =
                    ReadingStateDto::new(&book.book_id, book.created_date, Some(progress));
                Some(SyncResultDto::ChangedReadingState(WrappedReadingStateDto {
                    reading_state,
                }))
            }),
    );
    let books = Some(changes.read_list_books.as_slice());
    let tags = |read_lists: Vec<SyncPointReadList>, kind: fn(WrappedTagDto) -> SyncResultDto| {
        read_lists
            .into_iter()
            .map(move |read_list| kind(WrappedTagDto::new(&read_list, books)))
    };
    items.extend(tags(changes.read_lists_added, SyncResultDto::NewTag));
    items.extend(tags(changes.read_lists_changed, SyncResultDto::ChangedTag));
    items.extend(
        changes
            .read_lists_removed
            .iter()
            .map(|read_list| SyncResultDto::DeletedTag(WrappedTagDto::new(read_list, None))),
    );
    items
}

pub(super) async fn reading_state(
    State(state): State<KoboState>,
    Authenticated(user): Authenticated,
    Path((_key, book_id)): Path<(String, BookId)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    match state.kobo.reading_state(&user, &book_id).await {
        Ok(KoboReply::Local(reading)) => Ok(Json([ReadingStateDto::new(
            &book_id,
            reading.book_created,
            reading.progress.as_ref(),
        )])
        .into_response()),
        Ok(KoboReply::Store) => to_store(&state, Method::GET, &uri, &headers, Bytes::new()).await,
        Err(error) => Err(from_kobo(&error, uri.path())),
    }
}

pub(super) async fn update_reading_state(
    State(state): State<KoboState>,
    Authenticated(user): Authenticated,
    Extension(api_key): Extension<ApiKey>,
    Path((_key, book_id)): Path<(String, BookId)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response, ApiError> {
    // Read whatever the content type, as Kotlin does, and kept raw for the store.
    let update: ReadingStatesUpdateDto = serde_json::from_slice(&body).map_err(|error| {
        let message = format!("JSON parse error: {error}");
        ApiError::new(StatusCode::BAD_REQUEST, message, uri.path())
    })?;
    let device = R2Device {
        id: api_key.id.to_string(),
        name: api_key.comment,
    };
    let update = update.into_update();
    match state
        .kobo
        .update_reading_state(&user, &book_id, device, update)
        .await
    {
        Ok(KoboReply::Local(recorded)) => {
            Ok(Json(RequestResultDto::new(&book_id, recorded)).into_response())
        }
        Ok(KoboReply::Store) => to_store(&state, Method::PUT, &uri, &headers, body).await,
        Err(error) => Err(from_kobo(&error, uri.path())),
    }
}
