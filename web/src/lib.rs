use axum::http::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    HeaderName, HeaderValue, Method,
};
use axum_login::{
    tower_sessions::{Expiry, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use domain::jobs::{password_reset, session_reminder, Scheduler};
use domain::user::Backend;
use tower_sessions::ExpiredDeletion;
use tower_sessions_sqlx_store::PostgresStore;

pub use self::error::{Error, Result};
use log::*;
use meeting_ai::traits::{recording_bot, transcription as transcription_trait};
use sea_orm::DatabaseConnection;
use service::config::{ApiVersion, Config};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use time::Duration;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, CorsLayer};

mod controller;
mod error;
pub(crate) mod extractors;
pub(crate) mod middleware;
pub(crate) mod params;
pub(crate) mod protect;
mod router;
pub mod sse;

/// Web-layer application state that includes both infrastructure and domain concerns.
/// This wraps the service-level state and adds the event publisher for domain events.
#[derive(Clone)]
pub struct AppState {
    pub database_connection: Arc<DatabaseConnection>,
    pub config: Config,
    pub sse_manager: Arc<::sse::Manager>,
    pub event_publisher: Arc<domain::events::EventPublisher>,
    pub oauth_state_manager: meeting_auth::oauth::StateManager,
    pub recording_bot_provider: Option<Arc<dyn recording_bot::Provider>>,
    pub transcription_provider: Option<Arc<dyn transcription_trait::Provider>>,
}

impl AppState {
    pub fn new(
        service_state: service::AppState,
        sse_manager: Arc<::sse::Manager>,
        event_publisher: domain::events::EventPublisher,
        recording_bot_provider: Option<Arc<dyn recording_bot::Provider>>,
        transcription_provider: Option<Arc<dyn transcription_trait::Provider>>,
    ) -> Self {
        Self {
            database_connection: service_state.database_connection,
            config: service_state.config,
            sse_manager,
            event_publisher: Arc::new(event_publisher),
            oauth_state_manager: meeting_auth::oauth::StateManager::new(),
            recording_bot_provider,
            transcription_provider,
        }
    }

    pub fn db_conn_ref(&self) -> &DatabaseConnection {
        self.database_connection.as_ref()
    }
}

pub async fn init_server(app_state: AppState) -> Result<()> {
    // Session layer
    let session_store = PostgresStore::new(
        app_state
            .db_conn_ref()
            .get_postgres_connection_pool()
            .to_owned(),
    )
    .with_schema_name("refactor_platform") // FIXME: consolidate all schema strings into a config field with default option
    .unwrap()
    .with_table_name("authorized_sessions")
    .unwrap();

    session_store.migrate().await.unwrap();

    let deletion_task = tokio::task::spawn(
        session_store
            .clone()
            .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
    );

    // Recurring background work. Each job is a periodic sweep that re-derives what is
    // due from current rows; see `domain::jobs` for why that shape rather than a durable
    // queue. The session-deletion task above stays hand-rolled because it is
    // `tower_sessions`' own helper, not one of our jobs.
    let mut scheduler = Scheduler::new(
        Arc::clone(&app_state.database_connection),
        app_state.config.clone(),
    );
    scheduler.spawn(password_reset::Sweep::new());
    match session_reminder::Sweep::from_config(&app_state.config) {
        Some(sweep) => {
            scheduler.spawn(sweep);
        }
        None => info!(
            "SESSION_REMINDER_EMAIL_TEMPLATE_ID not set — upcoming-session reminders disabled"
        ),
    }
    let job_handles = scheduler.into_handles();

    // Background sweep of the user_lookup_attempts throttle table. Unlike the audit
    // tables it is written on every email lookup and read only within a one-hour
    // window, so without this it grows for the life of the deployment.
    //
    // Retention exceeds the rate-limit window on purpose: a sweep landing mid-window
    // must not delete rows the next check still needs to count.
    let user_lookup_sweep_task = tokio::task::spawn({
        let db = Arc::clone(&app_state.database_connection);
        async move {
            const SWEEP_INTERVAL: tokio::time::Duration = tokio::time::Duration::from_secs(60 * 60);
            const RETENTION_HOURS: i64 = 24;
            loop {
                tokio::time::sleep(SWEEP_INTERVAL).await;
                match domain::user_lookup::sweep_old_attempts(&db, RETENTION_HOURS).await {
                    Ok(deleted) if deleted > 0 => {
                        log::info!(
                            "[user-lookup-sweep] removed {deleted} attempt record(s) \
                             older than {RETENTION_HOURS}h"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!("[user-lookup-sweep] sweep iteration failed: {e:?}");
                    }
                }
            }
        }
    });

    let session_layer = SessionManagerLayer::new(session_store)
        // Get non-secure cookies for local testing, while production automatically gets secure cookies
        .with_secure(app_state.config.is_production())
        .with_same_site(tower_sessions::cookie::SameSite::Lax) // Assists in CSRF protection
        .with_expiry(Expiry::OnInactivity(Duration::seconds(
            app_state.config.backend_session_expiry_seconds as i64,
        )))
        // Save session on every request to reset the inactivity timer
        // This ensures active users stay logged in
        .with_always_save(true);

    // Auth service
    let backend = Backend::new(&app_state.database_connection);
    let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

    // These will probably come from app_state.config (command line)
    let host = app_state.config.interface.as_ref().unwrap();
    let port = app_state.config.port;

    app_state.config.log_non_secret_config();

    if app_state.config.is_production() {
        info!("Server starting... listening for internal connections on http://{host}:{port}");
        info!("External access available via HTTPS proxy at https://myrefactor.com");
    } else {
        info!("Server starting... listening for connections on http://{host}:{port}");
    }

    let server_url = format!("{host}:{port}");
    let listen_addr = SocketAddr::from_str(&server_url).unwrap();

    let listener = TcpListener::bind(listen_addr).await.unwrap();

    // Handle CORS origin configuration
    // If wildcard (*) is present, mirror request origin; otherwise use explicit list
    let has_wildcard = app_state
        .config
        .allowed_origins
        .iter()
        .any(|origin| origin == "*");

    // Mirror the request origin when wildcard "*" is configured to keep credentials enabled
    // SECURITY: Refuse wildcard CORS in production — mirror_request() with credentials
    // allows any origin to make authenticated API requests (CSRF/data-exfiltration risk)
    let allow_origin = if has_wildcard && app_state.config.is_production() {
        warn!(
            "ALLOWED_ORIGINS contains '*' in production — ignoring wildcard for security. \
             Set explicit origins instead."
        );
        AllowOrigin::list(Vec::<HeaderValue>::new())
    } else if has_wildcard {
        info!("Using mirrored CORS origin (allows all origins with credentials)");
        AllowOrigin::mirror_request()
    } else {
        let allowed_origins = app_state
            .config
            .allowed_origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect::<Vec<HeaderValue>>();
        AllowOrigin::list(allowed_origins)
    };

    let cors_layer = CorsLayer::new()
        .allow_methods([
            Method::DELETE,
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
        ])
        // Essential to allow credentials through a reverse proxy like nginx
        .allow_credentials(true)
        // Allow and expose the X-Version header across origins
        .allow_headers([
            ApiVersion::field_name().parse::<HeaderName>().unwrap(),
            AUTHORIZATION,
            CONTENT_TYPE,
            // Headers that nginx reverse proxy might forward
            "X-Forwarded-For".parse::<HeaderName>().unwrap(),
            "X-Forwarded-Proto".parse::<HeaderName>().unwrap(),
            "X-Real-IP".parse::<HeaderName>().unwrap(),
            "X-Request-ID".parse::<HeaderName>().unwrap(),
        ])
        .expose_headers([ApiVersion::field_name().parse::<HeaderName>().unwrap()])
        .allow_private_network(true)
        .allow_origin(allow_origin);

    axum::serve(
        listener,
        router::define_routes(app_state)
            .layer(cors_layer)
            .layer(auth_layer)
            // `into_make_service_with_connect_info` (not just `into_make_service`)
            // injects `ConnectInfo<SocketAddr>` into every request's extensions.
            // Required by `tower_governor`'s `SmartIpKeyExtractor`: when none of
            // `X-Forwarded-For` / `X-Real-IP` / `Forwarded` is set (i.e. local dev
            // without a proxy in front), the extractor falls back to the peer
            // SocketAddr from `ConnectInfo`. Without this, every `/password-reset/*`
            // request returns `500 "Unable To Extract Key!"` from the throttle
            // middleware before any route handler runs. See `web::middleware::throttle`.
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();

    let _res = deletion_task.await.unwrap();
    // No `let _res = …` on the job handles: each task's future returns `()`, so binding
    // it would trigger clippy's `let_unit_value` lint.
    for handle in job_handles {
        handle.await.unwrap();
    }
    user_lookup_sweep_task.await.unwrap();

    Ok(())
}
