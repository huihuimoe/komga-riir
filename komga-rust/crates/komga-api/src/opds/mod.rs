//! OPDS v1.2 and v2 catalogs.

mod common;
mod dto;
mod query;
mod v1;
mod v2;

pub(crate) use v2::authentication_required;

use std::ops::Deref;
use std::sync::Arc;

use axum::Router;
use axum::extract::FromRef;
use axum::middleware::from_fn_with_state;
use jiff::ZonedDateTime;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use komga_service::{
    ArtworkService, CatalogService, LibraryService, ReadListService, ReadProgressService,
    ReadingService, ReferentialService, SeriesCollectionService,
};
use komga_types::PageRequest;

use crate::auth::{authenticate_request, permit_anonymous};
use crate::mvc::jackson_date_time::iso_offset_date_time;
use crate::routes::ApiState;

#[derive(Clone)]
struct OpdsState(Arc<OpdsStateInner>);

struct OpdsStateInner {
    artwork: ArtworkService,
    catalog: CatalogService,
    collections: SeriesCollectionService,
    libraries: LibraryService,
    read_lists: ReadListService,
    read_progress: ReadProgressService,
    reading: ReadingService,
    referential: ReferentialService,
}

impl Deref for OpdsState {
    type Target = OpdsStateInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromRef<OpdsState> for ArtworkService {
    fn from_ref(state: &OpdsState) -> Self {
        state.artwork.clone()
    }
}

impl FromRef<OpdsState> for ReadProgressService {
    fn from_ref(state: &OpdsState) -> Self {
        state.read_progress.clone()
    }
}

impl FromRef<OpdsState> for ReadingService {
    fn from_ref(state: &OpdsState) -> Self {
        state.reading.clone()
    }
}

pub fn router(state: &ApiState) -> Router {
    Router::new()
        .merge(common::routes())
        .merge(v1::routes())
        .merge(v2::routes())
        .route_layer(from_fn_with_state(state.auth.clone(), authenticate_request))
        .merge(
            v2::public_routes()
                .route_layer(from_fn_with_state(state.auth.clone(), permit_anonymous)),
        )
        .with_state(OpdsState(Arc::new(OpdsStateInner {
            artwork: state.artwork.clone(),
            catalog: state.catalog.clone(),
            collections: state.collections.clone(),
            libraries: state.libraries.clone(),
            read_lists: state.read_lists.clone(),
            read_progress: state.read_progress.clone(),
            reading: state.reading.clone(),
            referential: state.referential.clone(),
        })))
}

/// `toZonedDateTime()` of a stored UTC date: the server's zone.
fn server_time(date: DateTime) -> String {
    iso_offset_date_time(
        &date
            .to_zoned_datetime(TimeZone::UTC)
            .expect("Stored dates are in range")
            .with_time_zone(TimeZone::system()),
    )
}

/// `ZonedDateTime.now()`.
fn now() -> String {
    iso_offset_date_time(&ZonedDateTime::now())
}

/// The `previous` and `next` links of `page` out of `total` items, to `href` with a `page`
/// parameter.
fn adjacent_pages(
    href: &str,
    page: PageRequest,
    total: u64,
) -> impl Iterator<Item = (&'static str, String)> {
    let separator = if href.contains('?') { '&' } else { '?' };
    let to = move |rel, number| (rel, format!("{href}{separator}page={number}"));
    let (previous, next) = match page {
        PageRequest::Paged { number, size } => (
            number.checked_sub(1),
            (u64::from(number) + 1 < total.div_ceil(u64::from(size))).then_some(number + 1),
        ),
        PageRequest::Unpaged => (None, None),
    };
    [
        previous.map(|number| to("previous", number)),
        next.map(|number| to("next", number)),
    ]
    .into_iter()
    .flatten()
}
