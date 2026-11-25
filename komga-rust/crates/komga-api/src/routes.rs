//! Every HTTP route of the server, built from one set of services.

use std::ops::Deref;
use std::sync::Arc;

use axum::Router;
use axum::extract::FromRef;
use komga_service::{
    AnnouncementService, ArtworkService, BookAnalysisService, BookImporter, CatalogService,
    ClientSettingsService, EventFeed, ExternalIdentityService, FileDeletion, FontService,
    HealthService, HistoryService, IdentityService, KoboSyncService, KoreaderSyncService,
    LibraryService, MetadataService, MetricsService, PageHashService, ReadListService,
    ReadProgressService, ReadingService, ReferentialService, ReleaseService,
    SeriesCollectionService, ServerSettingsService, SyncPointService, TaskService,
    TransientBookService,
};
use tower::Layer;

use crate::actuator::{self, BuildInfo, ServerControl};
use crate::auth::{self, AuthState};
use crate::kobo::{self, KoboStore};
use crate::sse::events::{self, SseConnections};
use crate::web_ui::{self, WebUi};
use crate::{koreader, mvc, opds, rest};

/// The address the running server actually uses, decided at startup.
#[derive(Debug, Clone)]
pub struct WebServerSettings {
    /// The static `server.port`, which defaults to 25600 like the baseline `application.yml`.
    pub configured_port: u16,
    pub effective_port: u16,
    /// The static `server.servlet.context-path`, if set.
    pub configured_context_path: Option<String>,
    /// Every route is served below it; empty at the root.
    pub context_path: Arc<str>,
    /// Peer IP allowlist for honoring `Forwarded` and `X-Forwarded-*` headers.
    pub trusted_proxies: mvc::servlet_uri::TrustedProxies,
}

/// The services behind the routes; each router takes what it needs.
pub struct ApiServices {
    pub auth: AuthState,
    pub announcements: AnnouncementService,
    pub releases: ReleaseService,
    pub build_info: BuildInfo,
    pub server_control: ServerControl,
    pub identity: IdentityService,
    pub analysis: BookAnalysisService,
    pub artwork: ArtworkService,
    pub catalog: CatalogService,
    pub client_settings: ClientSettingsService,
    pub collections: SeriesCollectionService,
    pub event_feed: EventFeed,
    pub external_identity: ExternalIdentityService,
    pub file_deletion: FileDeletion,
    pub fonts: FontService,
    pub health: HealthService,
    pub metrics: MetricsService,
    pub history: HistoryService,
    pub importer: BookImporter,
    pub kobo: KoboSyncService,
    pub kobo_store: KoboStore,
    pub koreader: KoreaderSyncService,
    pub libraries: LibraryService,
    pub metadata: MetadataService,
    pub page_hashes: PageHashService,
    pub read_lists: ReadListService,
    pub read_progress: ReadProgressService,
    pub reading: ReadingService,
    pub referential: ReferentialService,
    pub settings: ServerSettingsService,
    /// Closed on shutdown so that open event streams do not hold it.
    pub sse_connections: SseConnections,
    pub sync_points: SyncPointService,
    pub tasks: TaskService,
    pub transient_books: TransientBookService,
    pub web_server: WebServerSettings,
    pub web_ui: WebUi,
}

/// Shared handle to [`ApiServices`] so Axum's per-request state clone is a single atomic increment.
#[derive(Clone)]
pub(crate) struct ApiState(Arc<ApiServices>);

impl Deref for ApiState {
    type Target = ApiServices;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

macro_rules! impl_from_ref {
    ($($field:ident: $ty:ty),* $(,)?) => {
        $(
            impl FromRef<ApiState> for $ty {
                fn from_ref(state: &ApiState) -> Self {
                    state.$field.clone()
                }
            }
        )*
    };
}

impl_from_ref!(
    auth: AuthState,
    announcements: AnnouncementService,
    releases: ReleaseService,
    build_info: BuildInfo,
    server_control: ServerControl,
    identity: IdentityService,
    analysis: BookAnalysisService,
    artwork: ArtworkService,
    catalog: CatalogService,
    client_settings: ClientSettingsService,
    collections: SeriesCollectionService,
    event_feed: EventFeed,
    external_identity: ExternalIdentityService,
    file_deletion: FileDeletion,
    fonts: FontService,
    health: HealthService,
    metrics: MetricsService,
    history: HistoryService,
    importer: BookImporter,
    kobo: KoboSyncService,
    kobo_store: KoboStore,
    koreader: KoreaderSyncService,
    libraries: LibraryService,
    metadata: MetadataService,
    page_hashes: PageHashService,
    read_lists: ReadListService,
    read_progress: ReadProgressService,
    reading: ReadingService,
    referential: ReferentialService,
    settings: ServerSettingsService,
    sse_connections: SseConnections,
    sync_points: SyncPointService,
    tasks: TaskService,
    transient_books: TransientBookService,
    web_server: WebServerSettings,
    web_ui: WebUi,
);

pub fn router(services: ApiServices) -> Router {
    let state = &ApiState(Arc::new(services));
    let routes = Router::new()
        .merge(actuator::router(state))
        .merge(auth::router(state))
        .merge(rest::router(state))
        .merge(events::router(state))
        .merge(kobo::router(state))
        .merge(koreader::router(state))
        .merge(opds::router(state))
        .merge(web_ui::router(state))
        .layer(axum::middleware::from_fn(
            mvc::method_not_allowed::error_body,
        ))
        .layer(axum::middleware::from_fn(crate::error::negotiate))
        .layer(axum::middleware::from_fn(mvc::http_cache::layer))
        .layer(axum::middleware::from_fn_with_state(
            state.metrics.clone(),
            actuator::record_request,
        ));
    // Outside the router: the layer rewrites the path that routing then matches.
    let context = axum::middleware::from_fn_with_state(
        mvc::servlet_uri::LayerConfig {
            context_path: state.web_server.context_path.clone(),
            trusted_proxies: state.web_server.trusted_proxies.clone(),
        },
        mvc::servlet_uri::layer,
    );
    let options = axum::middleware::from_fn(mvc::method_not_allowed::options);
    Router::new().fallback_service(context.layer(options.layer(routes)))
}
