//! Kotlin `Opds2Controller`'s feeds.

mod catalog;
mod library;
mod list;

use komga_types::{BooleanOperator, Library, LibraryId, PageRequest, SeriesCondition};

use super::request::{Opds2, page_metadata};
use crate::error::ApiError;
use crate::opds::dto::feed::{Feed, FeedMetadata, Group};
use crate::opds::query::series_library;
use crate::opds::{OpdsState, now};
use crate::webpub::{WpLink, WpPublication};

pub(super) use catalog::{search, series};
pub(super) use library::{BookFeed, books, browse, latest_series, recommended};
pub(super) use list::{ListFeed, collection, lists, read_list};

/// `libraries` or `libraries/<id>`, the root of a library's feeds.
fn library_root(library: Option<&LibraryId>) -> String {
    match library {
        Some(id) => format!("libraries/{id}"),
        None => "libraries".to_owned(),
    }
}

/// Series that are not one-shots, of the library if any.
fn series_conditions(library: Option<&LibraryId>) -> Vec<SeriesCondition> {
    let mut conditions = vec![SeriesCondition::OneShot(BooleanOperator::IsFalse)];
    conditions.extend(library.map(series_library));
    conditions
}

/// The title and modification date of a library's feeds.
fn library_metadata(library: Option<&Library>, suffix: &str) -> FeedMetadata {
    let title = library.map_or("All libraries", |library| library.name.as_str());
    FeedMetadata {
        title: format!("{title}{suffix}"),
        modified: Some(now()),
        ..FeedMetadata::default()
    }
}

fn feed(metadata: FeedMetadata, links: Vec<WpLink>) -> Feed {
    Feed {
        metadata,
        links,
        ..Feed::default()
    }
}

fn paged_feed(
    request: &Opds2,
    mut metadata: FeedMetadata,
    href: String,
    items: usize,
    total: u64,
) -> Feed {
    metadata.page = Some(page_metadata(request.page, items, total));
    feed(metadata, request.paged_links(href, total))
}

fn group(title: &str, navigation: Vec<WpLink>, publications: Vec<WpPublication>) -> Group {
    Group {
        metadata: FeedMetadata::titled(title),
        links: Vec::new(),
        navigation,
        publications,
    }
}

/// `getLibraryNavigation`: the feeds of a library, collections and read lists only if it has
/// any.
async fn library_navigation(
    state: &OpdsState,
    request: &Opds2,
    library: Option<&LibraryId>,
) -> Result<Vec<WpLink>, ApiError> {
    let root = library_root(library);
    let link = |title: &str, path: String| WpLink {
        rel: Some("subsection"),
        ..request.navigation(title.to_owned(), &path)
    };
    let mut links = vec![
        link("Recommended", root.clone()),
        link("Browse", format!("{root}/browse")),
    ];
    let one = request.query_page(state, PageRequest::Paged { number: 0, size: 1 });
    if one.collections(library, None).await?.total_elements > 0 {
        links.push(link("Collections", format!("{root}/collections")));
    }
    if one.read_lists(library, None).await?.total_elements > 0 {
        links.push(link("Read Lists", format!("{root}/readlists")));
    }
    Ok(links)
}
