use axum::http::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    HeaderName, HeaderValue, Method,
};
use axum_login::{
    tower_sessions::{Expiry, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use entity_api::user::Backend;
use tower_sessions::session_store::ExpiredDeletion;
use tower_sessions_sqlx_store::PostgresStore;

pub use self::error::{Error, Result};
use log::*;
use service::{config::ApiVersion, AppState};
use std::net::SocketAddr;
use std::str::FromStr;
use time::Duration;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

mod controller;
mod error;
pub(crate) mod extractors;
pub(crate) mod protect;
mod router;

pub async fn init_server(app_state: AppState) -> Result<()> {
    info!(
        "Connecting to DB with URI: {}",
        app_state.config.database_uri()
    );
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

    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(1)));

    // Auth service
    let backend = Backend::new(&app_state.database_connection);
    let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

    // These will probably come from app_state.config (command line)
    let host = app_state.config.interface.as_ref().unwrap();
    let port = app_state.config.port;
    info!("Server starting... listening for connections on http://{host}:{port}");

    let server_url = format!("{host}:{port}");
    let listen_addr = SocketAddr::from_str(&server_url).unwrap();

    let listener = TcpListener::bind(listen_addr).await.unwrap();
    // Convert the type of the allow_origins Vec into a HeaderValue that the CorsLayer accepts
    let allowed_origins = app_state
        .config
        .allowed_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect::<Vec<HeaderValue>>();
    info!("allowed_origins: {:#?}", allowed_origins);

    let cors_layer = CorsLayer::new()
        .allow_methods([
            Method::DELETE,
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
        ])
        .allow_credentials(true)
        // Allow and expose the X-Version header across origins
        .allow_headers([
            ApiVersion::field_name().parse::<HeaderName>().unwrap(),
            AUTHORIZATION,
            CONTENT_TYPE,
        ])
        .expose_headers([ApiVersion::field_name().parse::<HeaderName>().unwrap()])
        .allow_private_network(true)
        .allow_origin(allowed_origins);

    axum::serve(
        listener,
        router::define_routes(app_state)
            .layer(cors_layer)
            .layer(auth_layer)
            .into_make_service(),
    )
    .await
    .unwrap();

    let _res = deletion_task.await.unwrap();

    Ok(())
}
