//! WebSocket entrypoint and per-connection actor.
//!
//! On upgrade, splits the socket and runs one task that owns the sink so direct
//! protocol replies and peer fan-out share a single writer. Authentication
//! happens per-document on the first `AuthToken` frame; sync frames that arrive
//! for a document before its token are held (bounded) and replayed once it
//! authenticates, matching the reference Hocuspocus server.

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

use crate::auth::{Authenticator, JwtAuthenticator};
use crate::config::Config;
use crate::document::{ConnectionId, Document};
use crate::protocol::{Body, Frame};
use crate::registry::DocumentRegistry;
use crate::rest;
use crate::storage::{PostgresStorage, Storage, StorageError};

/// Frames one connection may hold for a document that has not authenticated yet.
pub const PREAUTH_QUEUE_MAX_FRAMES: usize = 64;
/// Total encoded bytes those held frames may occupy per document.
pub const PREAUTH_QUEUE_MAX_BYTES: usize = 256 * 1024;
/// Distinct documents one connection may have awaiting a token at once. The
/// provider multiplexes a small fixed set, so this only bites an abuser.
pub const PREAUTH_QUEUE_MAX_DOCS: usize = 8;
/// Largest single WebSocket message accepted. Real frames are a few KiB; a
/// full-state `SyncStep2` for a large document is well under this. Replaces
/// axum's 64 MiB default on an endpoint reachable before authentication.
pub const MAX_WS_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

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
        await_shutdown_signal().await;
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

/// Resolve on the first shutdown signal: SIGINT (Ctrl-C) everywhere, or, on
/// unix, SIGTERM. `docker stop`/`restart` and `compose down` send SIGTERM, so
/// without the SIGTERM arm every deploy restart would SIGKILL the process
/// past its grace period with debounced writes unflushed.
async fn await_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            // Can't install the handler: never resolve this arm so Ctrl-C still
            // drives shutdown, rather than treating the failure as a signal.
            Err(e) => {
                warn!(error = %e, "failed to install SIGTERM handler; SIGTERM will not flush");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("SIGINT received; initiating graceful shutdown"),
        _ = terminate => info!("SIGTERM received; initiating graceful shutdown"),
    }
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
    ws.max_message_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_connection(socket, state))
}

/// Per-document authentication state on one connection. A document becomes
/// `Pending` the moment any frame names it and `Authed` on a valid token;
/// frames that arrive while `Pending` are held and replayed on auth.
enum DocAuth {
    Pending(PendingFrames),
    Authed,
}

/// Frames held for a document awaiting its token, capped by count and bytes.
/// `@hocuspocus/provider` sends awareness before `Auth` whenever its token
/// callback is slow, so refusing these would fail every slow-link client.
#[derive(Default)]
struct PendingFrames {
    frames: Vec<Body>,
    bytes: usize,
    /// Once tripped, nothing further is held until the document authenticates.
    overflowed: bool,
}

impl PendingFrames {
    /// Hold `body`, or report that a cap was exceeded. Overflow clears the
    /// queue so an unauthenticated peer cannot pin memory.
    fn push(&mut self, body: Body, wire_len: usize) -> Result<(), ()> {
        let over = self.overflowed
            || self.frames.len() >= PREAUTH_QUEUE_MAX_FRAMES
            || self.bytes + wire_len > PREAUTH_QUEUE_MAX_BYTES;
        if over {
            self.frames.clear();
            self.bytes = 0;
            self.overflowed = true;
            return Err(());
        }
        self.frames.push(body);
        self.bytes += wire_len;
        Ok(())
    }
}

/// Mutable per-connection state shared by the actor loop and frame dispatch.
struct Connection {
    auth: HashMap<String, DocAuth>,
    joined: HashMap<String, (Arc<Document>, ConnectionId)>,
    peers: StreamMap<String, BroadcastStream<Vec<u8>>>,
}

impl Connection {
    fn pending_doc_count(&self) -> usize {
        self.auth
            .values()
            .filter(|a| matches!(a, DocAuth::Pending(_)))
            .count()
    }
}

/// Per-connection actor. Owns the WS sink + stream and the per-doc broadcast
/// receivers. Multiplexes inbound frames, peer-published frames, and the
/// shutdown signal through a single `tokio::select!`, so writes from each
/// source are naturally serialized on the one sink.
async fn run_connection(socket: WebSocket, mut state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let mut conn = Connection {
        auth: HashMap::new(),
        joined: HashMap::new(),
        peers: StreamMap::new(),
    };

    loop {
        tokio::select! {
            inbound = stream.next() => {
                let Some(Ok(msg)) = inbound else { break };
                match msg {
                    Message::Binary(bytes) => match Frame::decode(&bytes) {
                        Ok(frame) => {
                            if dispatch_frame(&state, &mut sink, &mut conn, frame, bytes.len())
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
            peer = conn.peers.next(), if !conn.peers.is_empty() => {
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

    for (_, (doc, id)) in conn.joined.drain() {
        doc.leave(id);
    }
}

/// Encode and write one frame. `Err(())` means the sink stopped accepting.
async fn send_frame(
    sink: &mut SplitSink<WebSocket, Message>,
    name: &str,
    body: Body,
) -> Result<(), ()> {
    let bytes = Frame {
        name: name.to_string(),
        body,
    }
    .encode();
    sink.send(Message::Binary(bytes)).await.map_err(|_| ())
}

/// Pure per-frame dispatch. `Err(())` signals the actor loop to break (the sink
/// has stopped accepting writes, so the connection is effectively dead).
/// `wire_len` is the encoded size, used to bound the pre-auth queue.
async fn dispatch_frame(
    state: &AppState,
    sink: &mut SplitSink<WebSocket, Message>,
    conn: &mut Connection,
    frame: Frame,
    wire_len: usize,
) -> Result<(), ()> {
    let Frame { name, body } = frame;
    match body {
        Body::AuthToken(token) => {
            match state.authenticator.authenticate(&token, &name).await {
                Ok(_scope) => {
                    // Frames held while the token was in flight replay in arrival order.
                    let held = match conn.auth.insert(name.clone(), DocAuth::Authed) {
                        Some(DocAuth::Pending(pending)) => pending.frames,
                        _ => Vec::new(),
                    };
                    send_frame(sink, &name, Body::Authenticated("readwrite".to_string())).await?;
                    for held_body in held {
                        handle_sync_frame(state, sink, conn, &name, held_body).await?;
                    }
                }
                Err(e) => {
                    // Discard anything held under the failed attempt, but never
                    // demote a document this socket already authenticated. Reply
                    // without closing so the client sees the rejection before
                    // its own close handshake fires.
                    if matches!(conn.auth.get(&name), Some(DocAuth::Pending(_))) {
                        conn.auth.remove(&name);
                    }
                    let _ = send_frame(sink, &name, Body::PermissionDenied(e.to_string())).await;
                }
            }
        }
        body @ (Body::SyncStep1(_)
        | Body::SyncStep2(_)
        | Body::Update(_)
        | Body::Awareness(_)
        | Body::AwarenessQuery) => {
            if matches!(conn.auth.get(&name), Some(DocAuth::Authed)) {
                return handle_sync_frame(state, sink, conn, &name, body).await;
            }
            // Bound the number of names one un-authed socket can register, so
            // the per-document caps cannot be multiplied by inventing names.
            let is_new = !conn.auth.contains_key(&name);
            if is_new && conn.pending_doc_count() >= PREAUTH_QUEUE_MAX_DOCS {
                let reason = format!("document {name} requires authentication");
                let _ = send_frame(sink, &name, Body::PermissionDenied(reason)).await;
                return Ok(());
            }
            let entry = conn
                .auth
                .entry(name.clone())
                .or_insert_with(|| DocAuth::Pending(PendingFrames::default()));
            let DocAuth::Pending(pending) = entry else {
                return Ok(());
            };
            if pending.push(body, wire_len).is_err() {
                let reason = format!("document {name} requires authentication");
                let _ = send_frame(sink, &name, Body::PermissionDenied(reason)).await;
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

/// Apply one sync/awareness frame for an authenticated document, joining it
/// on first use (which mirrors Hocuspocus's on-connect handshake: our
/// `SyncStep1` so the client returns state we lack, then existing peers'
/// awareness so presence shows on load).
async fn handle_sync_frame(
    state: &AppState,
    sink: &mut SplitSink<WebSocket, Message>,
    conn: &mut Connection,
    name: &str,
    body: Body,
) -> Result<(), ()> {
    let (doc, id) = match conn.joined.get(name) {
        Some((doc, id)) => (doc.clone(), *id),
        None => match join_document(state, &mut conn.peers, name).await {
            Ok(pair) => {
                conn.joined.insert(name.to_string(), pair.clone());
                let mut join_frames = vec![pair.0.sync_step1()];
                join_frames.extend(pair.0.current_awareness_reply());
                for join_body in join_frames {
                    send_frame(sink, name, join_body).await?;
                }
                pair
            }
            Err(reason) => {
                let _ = send_frame(sink, name, Body::PermissionDenied(reason)).await;
                return Ok(());
            }
        },
    };

    match doc.handle(id, body).await {
        Ok(replies) => {
            for reply_body in replies {
                send_frame(sink, name, reply_body).await?;
            }
        }
        Err(e) => warn!(name = %name, error = %e, "doc.handle failed"),
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
