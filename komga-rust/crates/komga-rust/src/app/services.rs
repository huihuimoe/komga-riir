//! The services behind the HTTP routes, each built once.

use std::sync::Arc;

use anyhow::{Context, Result};
use komga_api::{
    ApiServices, AuthState, KoboStore, ServerControl, SseConnections, WebServerSettings,
};
use komga_db::Database;
use komga_scheduler::{TaskPoolSize, TaskQueue};
use komga_service::{
    AnnouncementService, ArtworkService, BookAnalysisService, BookImporter, CatalogService,
    ClientSettingsService, EventFeed, ExternalIdentityService, FileDeletion, FontService,
    FullTextSearch, HealthService, HistoryService, IdentityService, KoboSyncService,
    KoreaderSyncService, LibraryService, MetadataService, MetricsService, PageHashService,
    ReadListService, ReadProgressService, ReadingService, ReferentialService, ReleaseService,
    RememberMeSettings, SeriesCollectionService, ServerSettingsService, SessionRegistry,
    SignInPolicy, SyncPointService, TaskService, TransientBookService,
};

use super::Storage;
use crate::config::AppConfig;

pub(super) async fn api_services(
    storage: &Storage,
    tasks: &TaskQueue,
    task_pool_size: TaskPoolSize,
    web_server: WebServerSettings,
    reading: ReadingService,
    fonts: FontService,
    config: &AppConfig,
) -> Result<ApiServices> {
    let database = &storage.database;
    let search = &storage.search;
    let events = &storage.events;
    let letter_count_threshold = config.epub_divina_letter_count_threshold.value;
    let full_text = FullTextSearch::new(Arc::clone(&storage.search_index));
    let (auth, identity) = auth_state(storage, config).await?;
    let external_identity = external_identity(database, &identity, config).await?;
    let artwork = ArtworkService::new(database.clone(), tasks.clone(), events.clone());
    let read_progress = ReadProgressService::new(database.clone(), events.clone());
    let (kobo, kobo_store, koreader) = kobo_services(database, &artwork, &read_progress, config);
    let (announcements, releases, health, metrics) = server_info(storage, tasks, config);
    Ok(ApiServices {
        auth,
        announcements,
        identity,
        analysis: BookAnalysisService::new(
            database.clone(),
            tasks.clone(),
            letter_count_threshold,
            search.clone(),
            events.clone(),
        ),
        catalog: CatalogService::new(database.clone(), full_text.clone()),
        client_settings: ClientSettingsService::new(database.clone()),
        collections: SeriesCollectionService::new(
            database.clone(),
            search.clone(),
            full_text.clone(),
            events.clone(),
        ),
        event_feed: EventFeed::new(database.clone(), events.clone()),
        external_identity,
        file_deletion: FileDeletion::new(
            database.clone(),
            tasks.clone(),
            search.clone(),
            events.clone(),
        ),
        importer: BookImporter::new(
            database.clone(),
            tasks.clone(),
            search.clone(),
            events.clone(),
        ),
        kobo,
        kobo_store,
        koreader,
        libraries: LibraryService::new(
            database.clone(),
            tasks.clone(),
            search.clone(),
            events.clone(),
        ),
        metadata: MetadataService::new(
            database.clone(),
            tasks.clone(),
            reading.clone(),
            search.clone(),
            events.clone(),
        ),
        page_hashes: PageHashService::new(database.clone(), tasks.clone(), reading.clone()),
        read_lists: ReadListService::new(
            database.clone(),
            search.clone(),
            full_text,
            events.clone(),
        ),
        releases,
        build_info: crate::BUILD_INFO,
        server_control: ServerControl::new(config.log_file.value.clone()),
        referential: ReferentialService::new(database.clone()),
        settings: ServerSettingsService::new(database.clone(), task_pool_size),
        sse_connections: SseConnections::default(),
        sync_points: SyncPointService::new(database.clone()),
        transient_books: TransientBookService::new(
            database.clone(),
            reading.clone(),
            letter_count_threshold,
        ),
        artwork,
        fonts,
        health,
        metrics,
        history: HistoryService::new(database.clone()),
        read_progress,
        reading,
        tasks: TaskService::new(tasks.clone()),
        web_server,
        web_ui: super::web_ui::WEB_UI,
    })
}

/// The services reporting on the server and fetching the Komga website and GitHub.
fn server_info(
    storage: &Storage,
    tasks: &TaskQueue,
    config: &AppConfig,
) -> (
    AnnouncementService,
    ReleaseService,
    HealthService,
    MetricsService,
) {
    let database = &storage.database;
    let timeout = config.server_info_timeout;
    (
        AnnouncementService::new(database.clone(), config.announcements_url.clone(), timeout),
        ReleaseService::new(config.releases_url.clone(), timeout),
        HealthService::new(
            database.clone(),
            storage.tasks.clone(),
            config.config_dir.value.clone(),
        ),
        MetricsService::new(database.clone(), tasks.clone()),
    )
}

/// The services behind the Kobo and KOReader sync APIs.
fn kobo_services(
    database: &Database,
    artwork: &ArtworkService,
    read_progress: &ReadProgressService,
    config: &AppConfig,
) -> (KoboSyncService, KoboStore, KoreaderSyncService) {
    let kobo = KoboSyncService::new(
        database.clone(),
        artwork.clone(),
        read_progress.clone(),
        config.kobo_sync_item_limit.value.unsigned_abs(),
    );
    let store = KoboStore::new(config.kobo_store_url.clone(), config.kobo_store_timeout);
    let koreader = KoreaderSyncService::new(database.clone(), read_progress.clone());
    (kobo, store, koreader)
}

async fn external_identity(
    database: &Database,
    identity: &IdentityService,
    config: &AppConfig,
) -> Result<ExternalIdentityService> {
    let registrations = config
        .oauth2
        .client
        .resolve(&komga_oauth::ProviderClient::new())
        .await
        .context("Failed to resolve the OAuth2 client registrations")?;
    let policy = SignInPolicy {
        account_creation: config.oauth2.account_creation.value,
        oidc_email_verification: config.oauth2.oidc_email_verification.value,
    };
    ExternalIdentityService::load(database.clone(), identity.clone(), registrations, policy)
        .await
        .context("Failed to load the OAuth2 flow keys")
}

/// The authentication every protected router shares, and the identities behind it.
async fn auth_state(storage: &Storage, config: &AppConfig) -> Result<(AuthState, IdentityService)> {
    let remember_me_settings = RememberMeSettings::load(&storage.database, &storage.settings)
        .await
        .context("Failed to load remember-me settings")?;
    let sessions = SessionRegistry::with_ttl(config.session_timeout.value);
    let identity = IdentityService::new(
        storage.database.clone(),
        sessions.clone(),
        storage.events.clone(),
    );
    let auth = AuthState::new(identity.clone(), sessions, remember_me_settings);
    Ok((auth, identity))
}
