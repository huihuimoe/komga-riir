//! The request every OPDS 1.2 feed answers, and how it is answered.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use komga_types::{BookDetails, KomgaUser, PageRequest, PageResult, SeriesDetails};

use super::entries::Base;
use crate::accept;
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::media::media_ranges;
use crate::mvc::pageable::page_request;
use crate::mvc::request_params::RequestParams;
use crate::mvc::servlet_uri::RequestContext;
use crate::opds::dto::atom::{Entry, Feed};
use crate::opds::query::Query;
use crate::opds::{OpdsState, now};

pub(super) const PRODUCES: &[&str] = &["application/atom+xml", "application/xml", "text/xml"];

/// What every feed request carries.
pub(super) struct Catalog {
    pub(super) user: Arc<KomgaUser>,
    pub(super) base: Base,
    pub(super) params: RequestParams,
    pub(super) page: PageRequest,
    pub(super) path: String,
    pub(super) produced: &'static str,
}

impl<S: Send + Sync> FromRequestParts<S> for Catalog {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Authenticated(user) = Authenticated::from_request_parts(parts, state).await?;
        let produced =
            accept::produced(media_ranges(&parts.headers), PRODUCES).expect("A produces route");
        let params = RequestParams::parse(parts.uri.query());
        Ok(Self {
            user,
            base: Base::of(&RequestContext::from_extensions(&parts.extensions)),
            page: page_request(&params, false),
            params,
            path: parts.uri.path().to_owned(),
            produced,
        })
    }
}

impl Catalog {
    pub(super) fn respond(&self, body: Vec<u8>) -> Response {
        (
            [(CONTENT_TYPE, HeaderValue::from_static(self.produced))],
            body,
        )
            .into_response()
    }

    /// A feed of `entries` with its `self`, `start` and paging links to `href`.
    pub(super) fn feed(
        &self,
        id: &str,
        title: String,
        href: String,
        total: u64,
        entries: Vec<Entry>,
    ) -> Response {
        self.feed_updated(id, title, now(), href, total, entries)
    }

    pub(super) fn feed_updated(
        &self,
        id: &str,
        title: String,
        updated: String,
        href: String,
        total: u64,
        entries: Vec<Entry>,
    ) -> Response {
        let links = self.base.feed_links(href, self.page, total);
        let feed = Feed {
            id: id.to_owned(),
            title,
            updated,
            links,
            entries,
        };
        self.respond(feed.to_xml())
    }

    pub(super) fn series_feed(
        &self,
        id: &str,
        title: String,
        href: String,
        series: PageResult<SeriesDetails>,
    ) -> Response {
        let entries = series
            .items
            .into_iter()
            .map(|it| self.base.series(it))
            .collect();
        self.feed(id, title, href, series.total_elements, entries)
    }

    /// Entries for `books`, whose page streaming links need their page types.
    pub(super) async fn books(
        &self,
        state: &OpdsState,
        books: Vec<BookDetails>,
        series_prefix: bool,
    ) -> Result<Vec<Entry>, ApiError> {
        let page_types = state
            .reading
            .page_types(&books)
            .await
            .map_err(|error| ApiError::from_reading(&error, &self.path))?;
        let prefix = |book: &BookDetails| {
            if series_prefix {
                format!("{} {}: ", book.series_title, book.metadata.number)
            } else {
                String::new()
            }
        };
        Ok(self.base.books(books, &page_types, prefix))
    }

    pub(super) fn query<'a>(&'a self, state: &'a OpdsState) -> Query<'a> {
        Query {
            state,
            user: &self.user,
            page: self.page,
            path: &self.path,
        }
    }

    pub(super) fn catalog_error(&self) -> impl Fn(komga_service::CatalogError) -> ApiError + '_ {
        |error| ApiError::from_catalog(&error, &self.path)
    }
}
