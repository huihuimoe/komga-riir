//! The request every OPDS 2 feed answers, and how it is answered.

use std::sync::Arc;

use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use komga_types::{BookDetails, KomgaUser, Library, LibraryId, PageRequest, SeriesDetails};

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::pageable::page_request;
use crate::mvc::request_params::RequestParams;
use crate::mvc::servlet_uri::{RequestContext, encode_path};
use crate::opds::dto::feed::{Feed, PageMetadata};
use crate::opds::query::Query;
use crate::opds::{OpdsState, adjacent_pages};
use crate::webpub::{self, OPDS_JSON, WpLink, WpPublication};

/// What every feed request carries.
pub(super) struct Opds2 {
    pub(super) user: Arc<KomgaUser>,
    context: String,
    pub(super) params: RequestParams,
    pub(super) page: PageRequest,
    pub(super) path: String,
}

impl<S: Send + Sync> FromRequestParts<S> for Opds2 {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Authenticated(user) = Authenticated::from_request_parts(parts, state).await?;
        let params = RequestParams::parse(parts.uri.query());
        Ok(Self {
            user,
            context: RequestContext::from_extensions(&parts.extensions).url(),
            page: page_request(&params, false),
            params,
            path: parts.uri.path().to_owned(),
        })
    }
}

impl Opds2 {
    pub(super) fn query<'a>(&'a self, state: &'a OpdsState) -> Query<'a> {
        self.query_page(state, self.page)
    }

    pub(super) fn query_page<'a>(&'a self, state: &'a OpdsState, page: PageRequest) -> Query<'a> {
        Query {
            state,
            user: &self.user,
            page,
            path: &self.path,
        }
    }

    pub(super) fn href(&self, path: &str) -> String {
        format!("{}/opds/v2/{}", self.context, encode_path(path))
    }

    /// A link to another feed.
    pub(super) fn navigation(&self, title: String, path: &str) -> WpLink {
        WpLink {
            title: Some(title),
            href: Some(self.href(path)),
            media_type: Some(OPDS_JSON.to_owned()),
            ..WpLink::default()
        }
    }

    fn start(&self) -> WpLink {
        WpLink {
            rel: Some("start"),
            ..self.navigation("Home".to_owned(), "catalog")
        }
    }

    fn search(&self) -> WpLink {
        WpLink {
            title: Some("Search".to_owned()),
            rel: Some("search"),
            href: Some(format!("{}{{?query}}", self.href("search"))),
            media_type: Some(OPDS_JSON.to_owned()),
            templated: true,
            ..WpLink::default()
        }
    }

    /// `self`, `start` and `search`, without `self` when `href` is `None`.
    pub(super) fn links(&self, href: Option<String>) -> Vec<WpLink> {
        let this = href.map(|href| WpLink {
            rel: Some("self"),
            href: Some(href),
            ..WpLink::default()
        });
        this.into_iter()
            .chain([self.start(), self.search()])
            .collect()
    }

    /// [`links`](Self::links) and the paging links of a feed of `total` items at `href`.
    pub(super) fn paged_links(&self, href: String, total: u64) -> Vec<WpLink> {
        let pages: Vec<_> = adjacent_pages(&href, self.page, total)
            .map(|(rel, href)| WpLink {
                rel: Some(rel),
                href: Some(href),
                ..WpLink::default()
            })
            .collect();
        let mut links = self.links(Some(href));
        links.extend(pages);
        links
    }

    pub(super) fn publications(&self, books: &[BookDetails]) -> Vec<WpPublication> {
        books
            .iter()
            .map(|book| webpub::publication(&self.context, book))
            .collect()
    }

    pub(super) fn series(&self, series: Vec<SeriesDetails>) -> Vec<WpLink> {
        series
            .into_iter()
            .map(|it| self.navigation(it.metadata.title, &format!("series/{}", it.series.id)))
            .collect()
    }

    /// The library `id`, if given and the user can see it.
    pub(super) async fn library(
        &self,
        state: &OpdsState,
        id: Option<&LibraryId>,
    ) -> Result<Option<Library>, ApiError> {
        let Some(id) = id else {
            return Ok(None);
        };
        state
            .libraries
            .get(&self.user, id)
            .await
            .map(Some)
            .map_err(|error| ApiError::from_library(error, &self.path))
    }
}

/// Kotlin `FeedMetadataDto(page = …)`.
pub(super) fn page_metadata(page: PageRequest, items: usize, total: u64) -> PageMetadata {
    let (items_per_page, number) = match page {
        PageRequest::Paged { number, size } => (size, number),
        PageRequest::Unpaged => (u32::try_from(items).expect("A page fits in memory"), 0),
    };
    PageMetadata {
        items_per_page,
        current_page: number + 1,
        number_of_items: total,
    }
}

pub(super) fn respond(feed: &Feed) -> Response {
    (
        [(CONTENT_TYPE, HeaderValue::from_static(OPDS_JSON))],
        Json(feed),
    )
        .into_response()
}
