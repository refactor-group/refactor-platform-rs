//! WebSocket entrypoint and per-connection actor.
//!
//! On upgrade, splits the socket and runs one task that owns the sink so direct
//! protocol replies and peer fan-out share a single writer. Authentication
//! happens per-document on the first `AuthToken` frame; subsequent sync frames
//! for an un-authed document are rejected with `PermissionDenied`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamMap;
use tracing::{debug, info, warn};

use crate::auth::{Authenticator, JwtAuthenticator, Scope};
use crate::config::Config;
use crate::document::{ConnectionId, Document};
use crate::protocol::{Body, Frame};
use crate::registry::DocumentRegistry;
use crate::rest;
use crate::storage::{PostgresStorage, Storage, StorageError};

/// Shared, clone-cheap server state. Cloned by axum on every request via the
/// `State<AppState>` extractor; all heavyweight fields are `Arc`-shared.
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<DocumentRegistry>,
    pub storage: Arc<dyn Storage>,
    pub authenticator: Arc<dyn Authenticator>,
    /// Verbatim shared secret required on management REST endpoints. Compared
    /// byte-for-byte to the `Authorization` header (no `Bearer ` prefix).
    pub management_auth_key: Arc<str>,
    /// Receiver flipped to `true` by `serve` on shutdown signal. Per-connection
    /// actors `select!` on it so an idle WS loop wakes promptly on Ctrl-C.
    pub shutdown: watch::Receiver<bool>,
}

/// Fatal startup or runtime error from `serve`. Connection-level errors are
/// logged and do not propagate (one bad client must not stop the server).
#[derive(Debug, Error)]
pub enum ServeError {
    #[error("missing required secret: {0}")]
    MissingSecret(&'static str),
    #[error("storage init failed: {0}")]
    Storage(#[from] StorageError),
    #[error("bind {addr} failed: {source}")]
    Bind {
        addr: String,
        #[source]
        source: std::io::Error,
    },
    #[error("serve failed: {0}")]
    Serve(#[source] std::io::Error),
}

/// Boot the server from a fully-resolved `Config`. Refuses to start if either
/// shared secret is absent (an empty fallback would silently accept every token
/// and every management call). Installs Ctrl-C as the graceful-shutdown signal
/// and runs a final `flush_all` so debounced writes still in-flight at shutdown
/// land in storage before exit.
pub async fn serve(config: Config) -> Result<(), ServeError> {
    let jwt_key = config
        .jwt_signing_key()
        .ok_or(ServeError::MissingSecret("JWT_SIGNING_KEY"))?
        .to_owned();
    let mgmt_key = config
        .management_auth_key()
        .ok_or(ServeError::MissingSecret("MANAGEMENT_AUTH_KEY"))?
        .to_owned();

    let storage: Arc<dyn Storage> = Arc::new(
        PostgresStorage::connect_with_pool(
            config.database_url(),
            config.database_schema(),
            config.db_max_connections(),
            config.db_min_connections(),
        )
        .await?,
    );
    let registry = DocumentRegistry::new_with_debounce(
        storage.clone(),
        Duration::from_millis(config.persist_debounce_ms()),
    );
    let authenticator: Arc<dyn Authenticator> = Arc::new(JwtAuthenticator::new(jwt_key));

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let state = AppState {
        registry: registry.clone(),
        storage,
        authenticator,
        management_auth_key: Arc::from(mgmt_key),
        shutdown: shutdown_rx,
    };

    let router = build_router(state);
    let addr = config.bind_addr().to_string();
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|source| ServeError::Bind {
            addr: addr.clone(),
            source,
        })?;
    info!(addr = %addr, "docs-collab-server listening");

    let shutdown_signal = async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("ctrl-c received; initiating graceful shutdown");
        // Wake per-connection actors. Errors here mean every receiver has
        // already dropped, which is harmless.
        let _ = shutdown_tx.send(true);
    };

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .map_err(ServeError::Serve)?;

    // The listener has stopped accepting and connections have been signaled.
    // `Document::Drop` aborts the persist task WITHOUT flushing, so without this
    // pass any update still inside its debounce window would be lost on exit.
    if let Err(e) = registry.flush_all().await {
        warn!(error = %e, "shutdown flush_all reported an error");
    }
    info!("docs-collab-server stopped");
    Ok(())
}

/// Assemble the routed application without binding a port. Useful for tests
/// that want to drive the server in-process; `serve` is the production wrapper.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(ws_handler))
        .route(
            "/api/documents/:name",
            post(rest::create_document).delete(rest::delete_document),
        )
        .route("/health", get(health_handler))
        .with_state(state)
}

async fn health_handler() -> StatusCode {
    StatusCode::OK
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run_connection(socket, state))
}

/// Per-connection actor. Owns the WS sink + stream and the per-doc broadcast
/// receivers. Multiplexes inbound frames, peer-published frames, and the
/// shutdown signal through a single `tokio::select!`, so writes from each
/// source are naturally serialized on the one sink.
async fn run_connection(socket: WebSocket, mut state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let mut authed: HashMap<String, Scope> = HashMap::new();
    let mut joined: HashMap<String, (Arc<Document>, ConnectionId)> = HashMap::new();
    let mut peers: StreamMap<String, BroadcastStream<Vec<u8>>> = StreamMap::new();

    loop {
        tokio::select! {
            inbound = stream.next() => {
                let Some(Ok(msg)) = inbound else { break };
                match msg {
                    Message::Binary(bytes) => match Frame::decode(&bytes) {
                        Ok(frame) => {
                            if dispatch_frame(
                                &state,
                                &mut sink,
                                &mut authed,
                                &mut joined,
                                &mut peers,
                                frame,
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                        }
                        Err(e) => debug!(error = %e, "frame decode failed; ignoring"),
                    },
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) | Message::Text(_) => {}
                }
            }
            // Guarded so an empty StreamMap (which yields `None` immediately)
            // does not spin the select loop.
            peer = peers.next(), if !peers.is_empty() => {
                let Some((name, item)) = peer else { continue };
                let bytes = match item {
                    Ok(b) => b,
                    // CRDT reconverges on the next update; a dropped broadcast
                    // item is tolerable here.
                    Err(BroadcastStreamRecvError::Lagged(n)) => {
                        debug!(name = %name, dropped = n, "peer broadcast lagged; continuing");
                        continue;
                    }
                };
                if sink.send(Message::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            _ = state.shutdown.changed() => break,
        }
    }

    for (_, (doc, id)) in joined.drain() {
        doc.leave(id);
    }
}

/// Pure per-frame dispatch. `Err(())` signals the actor loop to break (the sink
/// has stopped accepting writes, so the connection is effectively dead).
async fn dispatch_frame(
    state: &AppState,
    sink: &mut SplitSink<WebSocket, Message>,
    authed: &mut HashMap<String, Scope>,
    joined: &mut HashMap<String, (Arc<Document>, ConnectionId)>,
    peers: &mut StreamMap<String, BroadcastStream<Vec<u8>>>,
    frame: Frame,
) -> Result<(), ()> {
    let Frame { name, body } = frame;
    match body {
        Body::AuthToken(token) => {
            match state.authenticator.authenticate(&token, &name).await {
                Ok(scope) => {
                    authed.insert(name.clone(), scope);
                    let reply = Frame {
                        name,
                        body: Body::Authenticated("readwrite".to_string()),
                    }
                    .encode();
                    sink.send(Message::Binary(reply)).await.map_err(|_| ())?;
                }
                Err(e) => {
                    // Reply with PermissionDenied; do NOT insert into `authed`.
                    // Leaving the socket open lets the client see the rejection
                    // before its own close handshake fires.
                    let reply = Frame {
                        name,
                        body: Body::PermissionDenied(e.to_string()),
                    }
                    .encode();
                    let _ = sink.send(Message::Binary(reply)).await;
                }
            }
        }
        body @ (Body::SyncStep1(_)
        | Body::SyncStep2(_)
        | Body::Update(_)
        | Body::Awareness(_)
        | Body::AwarenessQuery) => {
            if !authed.contains_key(&name) {
                let reply = Frame {
                    name: name.clone(),
                    body: Body::PermissionDenied(format!(
                        "document {name} requires authentication"
                    )),
                }
                .encode();
                let _ = sink.send(Message::Binary(reply)).await;
                return Ok(());
            }

            let (doc, id) = match joined.get(&name) {
                Some((doc, id)) => (doc.clone(), *id),
                None => match join_document(state, peers, &name).await {
                    Ok(pair) => {
                        joined.insert(name.clone(), pair.clone());
                        pair
                    }
                    Err(reason) => {
                        let reply = Frame {
                            name: name.clone(),
                            body: Body::PermissionDenied(reason),
                        }
                        .encode();
                        let _ = sink.send(Message::Binary(reply)).await;
                        return Ok(());
                    }
                },
            };

            match doc.handle(id, body).await {
                Ok(replies) => {
                    for reply_body in replies {
                        let bytes = Frame {
                            name: name.clone(),
                            body: reply_body,
                        }
                        .encode();
                        sink.send(Message::Binary(bytes)).await.map_err(|_| ())?;
                    }
                }
                Err(e) => warn!(name = %name, error = %e, "doc.handle failed"),
            }
        }
        // Server-bound or client-info only; no server action.
        Body::Stateless(_)
        | Body::SyncStatus(_)
        | Body::Close
        | Body::Authenticated(_)
        | Body::PermissionDenied(_) => {}
    }
    Ok(())
}

/// Load (or look up) the doc, join, and register the broadcast receiver on the
/// connection's per-doc fan-in map. Returns the strong `Arc<Document>` so the
/// caller keeps the doc loaded for the connection's lifetime.
async fn join_document(
    state: &AppState,
    peers: &mut StreamMap<String, BroadcastStream<Vec<u8>>>,
    name: &str,
) -> Result<(Arc<Document>, ConnectionId), String> {
    let doc = state.registry.get_or_load(name).await.map_err(|e| {
        warn!(name = %name, error = %e, "registry load failed");
        "server error loading document".to_string()
    })?;
    let (id, rx) = doc.join();
    peers.insert(name.to_string(), BroadcastStream::new(rx));
    Ok((doc, id))
}
