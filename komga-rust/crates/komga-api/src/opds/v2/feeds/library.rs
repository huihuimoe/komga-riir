//! Library-scoped OPDS 2 feeds: recommended, paged book feeds, latest series, and browse.

use axum::extract::{Path, State};
use axum::response::Response;
use komga_service::SeriesValue;
use komga_types::{FilterBy, LibraryId, PageRequest};

use super::{
    feed, group, library_metadata, library_navigation, library_root, paged_feed, series_conditions,
};
use crate::error::ApiError;
use crate::mvc::servlet_uri::encode_query_param;
use crate::opds::OpdsState;
use crate::opds::dto::feed::Group;
use crate::opds::query::{any_publisher, series_library};
use crate::opds::v2::request::{Opds2, respond};
use crate::webpub::{WpLink, WpPublication};

const RECOMMENDED_ITEMS: u32 = 5;

/// The catalog, or a library's recommended feed: the first books and series of its feeds.
pub async fn recommended(
    State(state): State<OpdsState>,
    id: Option<Path<LibraryId>>,
    request: Opds2,
) -> Result<Response, ApiError> {
    let id = id.map(|Path(id)| id);
    let library = request.library(&state, id.as_ref()).await?;
    let root = library_root(id.as_ref());
    let mut groups = Vec::new();
    if library.is_none() {
        groups.push(libraries_group(&state, &request).await?);
    }
    groups.extend(recommended_groups(&state, &request, id.as_ref(), &root).await?);

    let mut feed = feed(
        library_metadata(library.as_ref(), " - Recommended"),
        request.links(Some(request.href(&root))),
    );
    feed.navigation = library_navigation(&state, &request, id.as_ref()).await?;
    feed.groups = groups;
    Ok(respond(&feed))
}

async fn libraries_group(state: &OpdsState, request: &Opds2) -> Result<Group, ApiError> {
    let libraries = state
        .libraries
        .list(&request.user)
        .await
        .map_err(|error| ApiError::from_library(error, &request.path))?;
    let navigation = libraries
        .into_iter()
        .map(|library| request.navigation(library.name, &format!("libraries/{}", library.id)))
        .collect();
    Ok(Group {
        links: vec![WpLink {
            rel: Some("self"),
            href: Some(request.href("libraries")),
            ..WpLink::default()
        }],
        ..group("Libraries", navigation, Vec::new())
    })
}

async fn recommended_groups(
    state: &OpdsState,
    request: &Opds2,
    id: Option<&LibraryId>,
    root: &str,
) -> Result<Vec<Group>, ApiError> {
    let first = request.query_page(
        state,
        PageRequest::Paged {
            number: 0,
            size: RECOMMENDED_ITEMS,
        },
    );
    let keep_reading = first.keep_reading(id).await?.items;
    let on_deck = first.on_deck(id).await?.items;
    let latest_books = first.latest_books(id).await?.items;
    let latest_series = first.latest_series(series_conditions(id)).await?.items;

    let mut groups = Vec::new();
    for (title, path, items) in [
        ("Keep Reading", "keep-reading", keep_reading),
        ("On Deck", "on-deck", on_deck),
        ("Latest Books", "books/latest", latest_books),
    ] {
        if !items.is_empty() {
            let publications = request.publications(&items);
            groups.push(titled_group(
                request,
                root,
                title,
                path,
                Vec::new(),
                publications,
            ));
        }
    }
    if !latest_series.is_empty() {
        let navigation = request.series(latest_series);
        groups.push(titled_group(
            request,
            root,
            "Latest Series",
            "series/latest",
            navigation,
            Vec::new(),
        ));
    }
    Ok(groups)
}

fn titled_group(
    request: &Opds2,
    root: &str,
    title: &str,
    path: &str,
    navigation: Vec<WpLink>,
    publications: Vec<WpPublication>,
) -> Group {
    Group {
        links: vec![WpLink {
            rel: Some("self"),
            ..request.navigation(title.to_owned(), &format!("{root}/{path}"))
        }],
        ..group(title, navigation, publications)
    }
}

/// Which books a library's paged book feed lists.
#[derive(Clone, Copy)]
pub enum BookFeed {
    KeepReading,
    OnDeck,
    LatestBooks,
}

/// A library's keep reading, on deck, or latest books feed.
pub async fn books(
    kind: BookFeed,
    State(state): State<OpdsState>,
    id: Option<Path<LibraryId>>,
    request: Opds2,
) -> Result<Response, ApiError> {
    let id = id.map(|Path(id)| id);
    let library = request.library(&state, id.as_ref()).await?;
    let query = request.query(&state);
    let (suffix, path, books) = match kind {
        BookFeed::KeepReading => (
            " - Keep Reading",
            "keep-reading",
            query.keep_reading(id.as_ref()).await?,
        ),
        BookFeed::OnDeck => (" - On Deck", "on-deck", query.on_deck(id.as_ref()).await?),
        BookFeed::LatestBooks => (
            " - Latest Books",
            "books/latest",
            query.latest_books(id.as_ref()).await?,
        ),
    };
    let href = request.href(&format!("{}/{path}", library_root(id.as_ref())));
    let metadata = library_metadata(library.as_ref(), suffix);
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

pub async fn latest_series(
    State(state): State<OpdsState>,
    id: Option<Path<LibraryId>>,
    request: Opds2,
) -> Result<Response, ApiError> {
    let id = id.map(|Path(id)| id);
    let library = request.library(&state, id.as_ref()).await?;
    let series = request
        .query(&state)
        .latest_series(series_conditions(id.as_ref()))
        .await?;
    let href = request.href(&format!("{}/series/latest", library_root(id.as_ref())));
    let metadata = library_metadata(library.as_ref(), " - Latest Series");
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

/// A library's series by title, of any of the `publisher` parameters.
pub async fn browse(
    State(state): State<OpdsState>,
    id: Option<Path<LibraryId>>,
    request: Opds2,
) -> Result<Response, ApiError> {
    let id = id.map(|Path(id)| id);
    let library = request.library(&state, id.as_ref()).await?;
    let publishers: Vec<String> = request
        .params
        .list("publisher")
        .expect("Strings always parse")
        .unwrap_or_default();
    let mut conditions: Vec<_> = id.as_ref().map(series_library).into_iter().collect();
    conditions.extend(any_publisher(&publishers));
    let series = request.query(&state).series_by_title(conditions).await?;
    let browse = request.href(&format!("{}/browse", library_root(id.as_ref())));
    let href = if publishers.is_empty() {
        browse.clone()
    } else {
        let query: Vec<_> = publishers.iter().map(|it| publisher_query(it)).collect();
        format!("{browse}?{}", query.join("&"))
    };
    let publisher_links = publisher_links(&state, &request, id.as_ref(), &browse).await?;

    let metadata = library_metadata(library.as_ref(), "");
    let mut feed = paged_feed(
        &request,
        metadata,
        href,
        series.items.len(),
        series.total_elements,
    );
    feed.navigation = library_navigation(&state, &request, id.as_ref()).await?;
    feed.groups
        .push(group("Series", request.series(series.items), Vec::new()));
    if !publisher_links.is_empty() {
        feed.groups
            .push(group("Publisher", publisher_links, Vec::new()));
    }
    Ok(respond(&feed))
}

fn publisher_query(publisher: &str) -> String {
    format!("publisher={}", encode_query_param(publisher))
}

async fn publisher_links(
    state: &OpdsState,
    request: &Opds2,
    id: Option<&LibraryId>,
    browse: &str,
) -> Result<Vec<WpLink>, ApiError> {
    let filter = id.map(|id| FilterBy::Libraries([id.clone()].into()));
    let all_publishers = state
        .referential
        .series_values(
            &request.user,
            SeriesValue::Publisher,
            None,
            filter.as_ref(),
            PageRequest::Unpaged,
        )
        .await
        .map_err(|error| ApiError::internal_error(&request.path, error))?;
    Ok(all_publishers
        .items
        .into_iter()
        .map(|publisher| WpLink {
            href: Some(format!("{browse}?{}", publisher_query(&publisher))),
            ..request.navigation(publisher, "")
        })
        .collect())
}
