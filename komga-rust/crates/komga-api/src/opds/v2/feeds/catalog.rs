//! Series detail and catalog search OPDS 2 feeds.

use axum::extract::{Path, State};
use axum::response::Response;
use komga_types::{
    BookCondition, BooleanOperator, EqualityNullable, FilterBy, FilterTags, PageRequest,
    SeriesCondition, SeriesId, StringOperator,
};

use super::{feed, group, paged_feed};
use crate::error::ApiError;
use crate::mvc::servlet_uri::encode_query_param;
use crate::opds::dto::feed::{Facet, FeedMetadata};
use crate::opds::v2::request::{Opds2, respond};
use crate::opds::{OpdsState, now, server_time};
use crate::webpub::WpLink;

/// A series' books, with a facet per book tag; the `tag` parameter keeps the books with that
/// tag.
pub async fn series(
    State(state): State<OpdsState>,
    Path(id): Path<SeriesId>,
    request: Opds2,
) -> Result<Response, ApiError> {
    let series = state
        .catalog
        .series(&request.user, &id)
        .await
        .map_err(|error| ApiError::from_catalog(&error, &request.path))?;
    let tag = request.params.joined("tag");
    let conditions = tag
        .iter()
        .map(|tag| BookCondition::Tag(EqualityNullable::Is { value: tag.clone() }))
        .collect();
    let books = request.query(&state).series_books(&id, conditions).await?;
    let path = format!("series/{id}");
    let mut href = request.href(&path);
    if let Some(tag) = &tag {
        href = format!("{href}?tag={}", encode_query_param(tag));
    }
    let tag_links = series_tag_links(&state, &request, &id, &path, tag.as_deref()).await?;
    let description = if series.metadata.summary.trim().is_empty() {
        series.books_metadata.summary
    } else {
        series.metadata.summary
    };
    let metadata = FeedMetadata {
        title: series.metadata.title,
        modified: Some(server_time(series.last_modified)),
        description: Some(description),
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
    if !tag_links.is_empty() {
        feed.facets.push(Facet {
            metadata: FeedMetadata::titled("Tag"),
            links: tag_links,
        });
    }
    Ok(respond(&feed))
}

async fn series_tag_links(
    state: &OpdsState,
    request: &Opds2,
    id: &SeriesId,
    path: &str,
    selected_tag: Option<&str>,
) -> Result<Vec<WpLink>, ApiError> {
    let tags = state
        .referential
        .tags(
            &request.user,
            None,
            FilterTags::Book,
            Some(&FilterBy::Series([id.clone()].into())),
            PageRequest::Unpaged,
        )
        .await
        .map_err(|error| ApiError::internal_error(&request.path, error))?;
    let base_href = request.href(path);
    Ok(tags
        .items
        .into_iter()
        .map(|it| WpLink {
            rel: (selected_tag == Some(it.as_str())).then_some("self"),
            href: Some(format!("{base_href}?tag={}", encode_query_param(&it))),
            ..request.navigation(it, "")
        })
        .collect())
}

/// The first series, books, collections and read lists whose title contains every term of
/// `query`.
pub async fn search(State(state): State<OpdsState>, request: Opds2) -> Result<Response, ApiError> {
    let query = request.params.joined("query").unwrap_or_default();
    let terms: Vec<_> = query.split_whitespace().collect();
    let first = request.query_page(
        &state,
        PageRequest::Paged {
            number: 0,
            size: 20,
        },
    );
    let series = first
        .series_by_title(title_series_conditions(&terms))
        .await?
        .items;
    let books = first.books_titled(&terms).await?.items;
    let search = Some(query.as_str()).filter(|query| !query.is_empty());
    let collections = first.collections(None, search).await?.items;
    let read_lists = first.read_lists(None, search).await?.items;

    let mut feed = feed(
        FeedMetadata {
            title: "Search results".to_owned(),
            modified: Some(now()),
            ..FeedMetadata::default()
        },
        request.links(None),
    );
    if !series.is_empty() {
        feed.groups
            .push(group("Series", request.series(series), Vec::new()));
    }
    if !books.is_empty() {
        feed.groups
            .push(group("Books", Vec::new(), request.publications(&books)));
    }
    if !collections.is_empty() {
        let links = collections
            .into_iter()
            .map(|it| request.navigation(it.name, &format!("collections/{}", it.id)))
            .collect();
        feed.groups.push(group("Collections", links, Vec::new()));
    }
    if !read_lists.is_empty() {
        let links = read_lists
            .into_iter()
            .map(|it| request.navigation(it.name, &format!("readlists/{}", it.id)))
            .collect();
        feed.groups.push(group("Read Lists", links, Vec::new()));
    }
    Ok(respond(&feed))
}

fn title_series_conditions(terms: &[&str]) -> Vec<SeriesCondition> {
    let mut conditions: Vec<_> = terms
        .iter()
        .map(|term| {
            SeriesCondition::Title(StringOperator::Contains {
                value: (*term).to_owned(),
            })
        })
        .collect();
    conditions.push(SeriesCondition::OneShot(BooleanOperator::IsFalse));
    conditions
}
