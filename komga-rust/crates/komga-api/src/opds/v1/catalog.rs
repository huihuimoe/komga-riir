//! Kotlin `OpdsController`'s feeds.

use axum::extract::{Path, State};
use axum::response::Response;
use komga_service::SeriesValue;
use komga_types::{
    CollectionId, LibraryId, PageRequest, ReadListId, SeriesCondition, SeriesId, StringOperator,
};

use super::request::Catalog;
use crate::error::ApiError;
use crate::mvc::servlet_uri::encode_query_param;
use crate::opds::dto::atom::{self, Entry, Feed, Link};
use crate::opds::query::{any_publisher, series_library};
use crate::opds::{OpdsState, now, server_time};

pub(super) async fn catalog(request: Catalog) -> Response {
    let base = &request.base;
    let entries = [
        (
            "Keep Reading",
            "keepReading",
            "Continue reading your in progress books",
            "keep-reading",
        ),
        ("On Deck", "ondeck", "Browse what to read next", "ondeck"),
        ("All series", "allSeries", "Browse by series", "series"),
        (
            "Latest series",
            "latestSeries",
            "Browse latest series",
            "series/latest",
        ),
        (
            "Latest books",
            "latestBooks",
            "Browse latest books",
            "books/latest",
        ),
        (
            "All libraries",
            "allLibraries",
            "Browse by library",
            "libraries",
        ),
        (
            "All collections",
            "allCollections",
            "Browse by collection",
            "collections",
        ),
        (
            "All read lists",
            "allReadLists",
            "Browse by read lists",
            "readlists",
        ),
        (
            "All publishers",
            "allPublishers",
            "Browse by publishers",
            "publishers",
        ),
    ]
    .into_iter()
    .map(|(title, id, content, path)| Entry {
        content: content.to_owned(),
        ..base.subsection(title.to_owned(), now(), id.to_owned(), path)
    })
    .collect();
    let links = vec![
        base.navigation("self", "catalog"),
        base.navigation("start", "catalog"),
        Link::new(
            "application/opensearchdescription+xml",
            "search",
            base.href("search"),
        ),
        Link::new("application/opds+json", "alternate", base.v2_catalog()),
    ];
    let feed = Feed {
        id: "root".to_owned(),
        title: "Komga OPDS catalog".to_owned(),
        updated: now(),
        links,
        entries,
    };
    request.respond(feed.to_xml())
}

pub(super) async fn search(request: Catalog) -> Response {
    request.respond(atom::open_search(&request.base.search_template()))
}

pub(super) async fn on_deck(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let books = request.query(&state).on_deck(None).await?;
    let total = books.total_elements;
    let entries = request.books(&state, books.items, true).await?;
    let href = request.base.href("ondeck");
    Ok(request.feed("ondeck", "On Deck".to_owned(), href, total, entries))
}

pub(super) async fn keep_reading(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let books = request.query(&state).keep_reading(None).await?;
    let total = books.total_elements;
    let entries = request.books(&state, books.items, true).await?;
    let href = request.base.href("keep-reading");
    Ok(request.feed(
        "keepReading",
        "Keep Reading".to_owned(),
        href,
        total,
        entries,
    ))
}

pub(super) async fn latest_books(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let books = request.query(&state).latest_books(None).await?;
    let total = books.total_elements;
    let entries = request.books(&state, books.items, true).await?;
    let href = request.base.href("books/latest");
    Ok(request.feed(
        "latestBooks",
        "Latest books".to_owned(),
        href,
        total,
        entries,
    ))
}

/// The series search; its title search does not use the full-text index.
pub(super) async fn series(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let search = request.params.joined("search");
    let publishers: Vec<String> = request
        .params
        .list("publisher")
        .expect("Strings always parse")
        .unwrap_or_default();
    let mut conditions = Vec::new();
    let mut query = Vec::new();
    if let Some(search) = &search {
        conditions.push(SeriesCondition::Title(StringOperator::Contains {
            value: search.clone(),
        }));
        query.push(format!("search={}", encode_query_param(search)));
    }
    conditions.extend(any_publisher(&publishers));
    query.extend(
        publishers
            .iter()
            .map(|publisher| format!("publisher={}", encode_query_param(publisher))),
    );
    let series = request.query(&state).series_by_title(conditions).await?;
    let mut href = request.base.href("series");
    if !query.is_empty() {
        href = format!("{href}?{}", query.join("&"));
    }
    let title = match search.filter(|search| !search.trim().is_empty()) {
        Some(search) => format!("Series search for: {search}"),
        None => "All series".to_owned(),
    };
    Ok(request.series_feed("allSeries", title, href, series))
}

pub(super) async fn latest_series(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let series = request.query(&state).latest_series(Vec::new()).await?;
    let href = request.base.href("series/latest");
    Ok(request.series_feed("latestSeries", "Latest series".to_owned(), href, series))
}

pub(super) async fn libraries(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let libraries = state
        .libraries
        .list(&request.user)
        .await
        .map_err(|error| ApiError::from_library(error, &request.path))?;
    let entries = libraries
        .into_iter()
        .map(|library| {
            let path = format!("libraries/{}", library.id);
            request
                .base
                .subsection(library.name, now(), library.id.to_string(), &path)
        })
        .collect();
    let href = request.base.href("libraries");
    let feed = Feed {
        id: "allLibraries".to_owned(),
        title: "All libraries".to_owned(),
        updated: now(),
        links: request.base.feed_links(href, PageRequest::Unpaged, 0),
        entries,
    };
    Ok(request.respond(feed.to_xml()))
}

pub(super) async fn collections(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let collections = request.query(&state).collections(None, None).await?;
    let entries = collections
        .items
        .iter()
        .map(|collection| {
            let id = collection.id.to_string();
            request.base.subsection(
                collection.name.clone(),
                server_time(collection.last_modified_date),
                id.clone(),
                &format!("collections/{id}"),
            )
        })
        .collect();
    let href = request.base.href("collections");
    Ok(request.feed(
        "allCollections",
        "All collections".to_owned(),
        href,
        collections.total_elements,
        entries,
    ))
}

pub(super) async fn read_lists(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let read_lists = request.query(&state).read_lists(None, None).await?;
    let entries = read_lists
        .items
        .iter()
        .map(|read_list| {
            let id = read_list.id.to_string();
            request.base.subsection(
                read_list.name.clone(),
                server_time(read_list.last_modified_date),
                id.clone(),
                &format!("readlists/{id}"),
            )
        })
        .collect();
    let href = request.base.href("readlists");
    Ok(request.feed(
        "allReadLists",
        "All read lists".to_owned(),
        href,
        read_lists.total_elements,
        entries,
    ))
}

pub(super) async fn publishers(
    State(state): State<OpdsState>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let publishers = state
        .referential
        .series_values(
            &request.user,
            SeriesValue::Publisher,
            None,
            None,
            request.page,
        )
        .await
        .map_err(|error| ApiError::internal_error(&request.path, error))?;
    let series = request.base.href("series");
    let entries = publishers
        .items
        .iter()
        .map(|publisher| {
            let encoded = encode_query_param(publisher);
            Entry {
                links: vec![Link::new(
                    atom::NAVIGATION,
                    "subsection",
                    format!("{series}?publisher={encoded}"),
                )],
                ..request.base.subsection(
                    publisher.clone(),
                    now(),
                    format!("publisher:{encoded}"),
                    "",
                )
            }
        })
        .collect();
    let href = request.base.href("publishers");
    Ok(request.feed(
        "allPublishers",
        "All publishers".to_owned(),
        href,
        publishers.total_elements,
        entries,
    ))
}

pub(super) async fn one_series(
    State(state): State<OpdsState>,
    Path(id): Path<SeriesId>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let series = state
        .catalog
        .series(&request.user, &id)
        .await
        .map_err(request.catalog_error())?;
    let books = request.query(&state).series_books(&id, Vec::new()).await?;
    let total = books.total_elements;
    let entries = request.books(&state, books.items, false).await?;
    let href = request.base.href(&format!("series/{id}"));
    let updated = server_time(series.last_modified);
    Ok(request.feed_updated(
        id.as_str(),
        series.metadata.title,
        updated,
        href,
        total,
        entries,
    ))
}

pub(super) async fn library(
    State(state): State<OpdsState>,
    Path(id): Path<LibraryId>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let library = state
        .libraries
        .get(&request.user, &id)
        .await
        .map_err(|error| ApiError::from_library(error, &request.path))?;
    let series = request
        .query(&state)
        .series_by_title(vec![series_library(&id)])
        .await?;
    let href = request.base.href(&format!("libraries/{id}"));
    Ok(request.series_feed(id.as_str(), library.name, href, series))
}

pub(super) async fn collection(
    State(state): State<OpdsState>,
    Path(id): Path<CollectionId>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let collection = state
        .collections
        .get(&request.user, &id)
        .await
        .map_err(|error| ApiError::from_collection(&error, &request.path))?;
    let series = request.query(&state).collection_series(&id).await?;
    let href = request.base.href(&format!("collections/{id}"));
    Ok(request.series_feed(id.as_ref(), collection.name, href, series))
}

pub(super) async fn read_list(
    State(state): State<OpdsState>,
    Path(id): Path<ReadListId>,
    request: Catalog,
) -> Result<Response, ApiError> {
    let read_list = state
        .read_lists
        .get(&request.user, &id)
        .await
        .map_err(|error| ApiError::from_read_list(&error, &request.path))?;
    let books = request.query(&state).read_list_books(&id).await?;
    let total = books.total_elements;
    let entries = request.books(&state, books.items, true).await?;
    let href = request.base.href(&format!("readlists/{id}"));
    Ok(request.feed(id.as_ref(), read_list.name, href, total, entries))
}
