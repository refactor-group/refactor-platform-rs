//use axum::{BoxError, error_handling::HandleErrorLayer, http::StatusCode};
use axum_login::{AuthManagerLayerBuilder, login_required, tower_sessions::{Expiry, MemoryStore, SessionManagerLayer}};
use entity_api::user::Backend;
pub use self::error::{Error, Result};
use log::*;
use service::AppState;
use std::net::SocketAddr;
use std::str::FromStr;
use time::Duration;
use tokio::net::TcpListener;
//use tower::ServiceBuilder;


mod controller;
mod error;
mod router;

pub async fn init_server(app_state: AppState) -> Result<()> {
    // Session layer
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(1)));

    // Auth service
    let backend = Backend::new(app_state.db_conn_ref().unwrap());
    let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();
    // let auth_service = ServiceBuilder::new()
    //     .layer(HandleErrorLayer::new(|_: BoxError| async {
    //         StatusCode::BAD_REQUEST
    //     }))
    //     .layer(AuthManagerLayerBuilder::new(auth_backend, session_layer).build());

    // These will probably come from app_state.config (command line)
    let host = app_state.config.interface.as_ref().unwrap();
    let port = app_state.config.port;
    let server_url = format!("{host}:{port}");

    let listen_addr = SocketAddr::from_str(&server_url).unwrap();

    info!("Server starting... listening for connections on http://{host}:{port}");

    let listener = TcpListener::bind(listen_addr).await.unwrap();
    axum::serve(listener,
        router::define_routes(app_state)
        .route_layer(login_required!(Backend, login_url = "/login"))
        .layer(auth_layer))
        .await
        .unwrap();

    Ok(())
}
