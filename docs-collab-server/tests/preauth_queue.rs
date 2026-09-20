//! Frozen regression tests for frames that arrive before a document's `AuthToken`.
//!
//! `@hocuspocus/provider` marks itself connected, then `await`s its token
//! callback before sending `Auth`. Any awareness change in that window is sent
//! immediately, so on a slow link an `Awareness` or `SyncStep1` frame reaches
//! the server ahead of the token. The reference Hocuspocus server queues such
//! frames per document and replays them once that document authenticates; a
//! server that instead answers `PermissionDenied` makes the provider raise
//! `authenticationFailed` and the editor gives up (rs#413).
//!
//! These tests boot the real router on an ephemeral port and speak the wire
//! protocol over a real socket with real HS256 tokens, so they exercise the
//! exact path the browser hits.

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use docs_collab_server::ws::{
    PREAUTH_QUEUE_MAX_BYTES, PREAUTH_QUEUE_MAX_DOCS, PREAUTH_QUEUE_MAX_FRAMES,
};
use docs_collab_server::{
    build_router, AppState, Body, DocumentRegistry, Frame, JwtAuthenticator, MemoryStorage, Storage,
};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use yrs::sync::{Awareness, AwarenessUpdate};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ClientID, Doc, StateVector};

const SECRET: &str = "preauth-queue-test-secret";
const SCOPE: &str = "org.rel.*";
const DOC_A: &str = "org.rel.aaaaaaaa-0000-0000-0000-000000000000-v0";
const DOC_B: &str = "org.rel.bbbbbbbb-0000-0000-0000-000000000000-v0";

/// Long enough for a loopback round trip under CI load; short enough that a
/// "nothing must arrive" assertion does not dominate the suite.
const QUIET: Duration = Duration::from_millis(400);
const REPLY: Duration = Duration::from_secs(3);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Mirrors `domain/src/jwt/claims.rs` so the token shape matches production.
#[derive(Serialize)]
struct Claims {
    exp: usize,
    iat: usize,
    ndf: usize,
    iss: String,
    sub: String,
    aud: String,
    #[serde(rename = "allowedDocumentNames")]
    allowed_document_names: Vec<String>,
}

fn mint(secret: &str) -> String {
    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        exp: (Utc::now() + ChronoDuration::hours(1)).timestamp() as usize,
        iat: now,
        ndf: now,
        iss: "https://refactorcoach.com".into(),
        sub: DOC_A.into(),
        aud: "tiptap_app_id_value".into(),
        allowed_document_names: vec![SCOPE.into()],
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("mint test token")
}

/// In-process server. Holds the shutdown sender so the per-connection actors'
/// `shutdown.changed()` arm does not fire the moment the sender would drop.
struct TestServer {
    url: String,
    _shutdown: watch::Sender<bool>,
}

async fn start_server() -> TestServer {
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let state = AppState {
        registry: DocumentRegistry::new(storage.clone()),
        storage,
        authenticator: Arc::new(JwtAuthenticator::new(SECRET)),
        management_auth_key: Arc::from("unused-in-these-tests"),
        shutdown: shutdown_rx,
    };
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(axum::serve(listener, build_router(state)).into_future());
    TestServer {
        url: format!("ws://{addr}/"),
        _shutdown: shutdown_tx,
    }
}

async fn connect(server: &TestServer) -> Socket {
    let (ws, _) = tokio_tungstenite::connect_async(&server.url)
        .await
        .expect("connect to in-process server");
    ws
}

async fn send(ws: &mut Socket, name: &str, body: Body) {
    let bytes = Frame {
        name: name.into(),
        body,
    }
    .encode();
    ws.send(Message::Binary(bytes)).await.expect("send frame");
}

/// Next binary frame within `wait`, or `None` if the socket stays quiet.
async fn recv(ws: &mut Socket, wait: Duration) -> Option<Frame> {
    loop {
        let msg = match timeout(wait, ws.next()).await {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(e))) => panic!("socket error: {e}"),
            Ok(None) => panic!("server closed the socket"),
            Err(_) => return None,
        };
        if let Message::Binary(bytes) = msg {
            return Some(Frame::decode(&bytes).expect("decode server frame"));
        }
    }
}

/// Collect frames until `stop` matches or the socket goes quiet for `wait`.
/// Returns everything seen, including the matching frame.
async fn recv_until(ws: &mut Socket, wait: Duration, stop: impl Fn(&Frame) -> bool) -> Vec<Frame> {
    let mut seen = Vec::new();
    while let Some(frame) = recv(ws, wait).await {
        let done = stop(&frame);
        seen.push(frame);
        if done {
            break;
        }
    }
    seen
}

fn step1() -> Body {
    Body::SyncStep1(StateVector::default())
}

/// A client-side awareness update carrying a distinctive local state, plus the
/// client id it was issued under so a peer can prove it was relayed.
fn awareness_update() -> (ClientID, Body) {
    let awareness = Awareness::new(Doc::new());
    awareness.set_local_state_raw(r#"{"presence":"preauth-test"}"#);
    let client_id = awareness.client_id();
    let raw = awareness.update().expect("encode awareness").encode_v1();
    let update = AwarenessUpdate::decode_v1(&raw).expect("decode awareness");
    (client_id, Body::Awareness(update))
}

fn is_denied(frame: &Frame) -> bool {
    matches!(frame.body, Body::PermissionDenied(_))
}

fn is_step2_for(name: &str) -> impl Fn(&Frame) -> bool + '_ {
    move |f| f.name == name && matches!(f.body, Body::SyncStep2(_))
}

fn assert_none_denied(frames: &[Frame]) {
    let denied: Vec<_> = frames.iter().filter(|f| is_denied(f)).collect();
    assert!(
        denied.is_empty(),
        "server must not reply PermissionDenied to frames that merely arrived before Auth; got {denied:?}"
    );
}

async fn authenticate(ws: &mut Socket, name: &str) {
    send(ws, name, Body::AuthToken(mint(SECRET))).await;
    let first = recv(ws, REPLY).await.expect("auth reply");
    assert_eq!(first.name, name);
    assert!(
        matches!(first.body, Body::Authenticated(_)),
        "expected Authenticated, got {:?}",
        first.body
    );
}

/// The core regression: a sync frame ahead of the token must be held, not
/// refused, and answered once the token lands.
#[tokio::test]
async fn sync_step1_sent_before_auth_is_answered_after_auth() {
    let server = start_server().await;
    let mut ws = connect(&server).await;

    send(&mut ws, DOC_A, step1()).await;
    assert_eq!(
        recv(&mut ws, QUIET).await,
        None,
        "a pre-auth SyncStep1 must be queued silently, not answered"
    );

    send(&mut ws, DOC_A, Body::AuthToken(mint(SECRET))).await;
    let frames = recv_until(&mut ws, REPLY, is_step2_for(DOC_A)).await;

    let first = frames.first().expect("server must reply to the token");
    assert!(
        matches!(first.body, Body::Authenticated(_)),
        "the first frame after a valid token must be Authenticated, got {:?}",
        first.body
    );
    assert_none_denied(&frames);
    assert!(
        frames.iter().any(is_step2_for(DOC_A)),
        "the queued SyncStep1 must be replayed and produce a SyncStep2; got {frames:?}"
    );
}

/// A silently-dropping "fix" would pass the test above. This one proves the
/// queued frame is actually applied: a peer already on the document must see
/// the awareness that was sent before the sender authenticated.
#[tokio::test]
async fn queued_awareness_is_applied_and_reaches_peers() {
    let server = start_server().await;

    let mut peer = connect(&server).await;
    authenticate(&mut peer, DOC_A).await;
    send(&mut peer, DOC_A, step1()).await;
    // Drain the peer's own join handshake so only the relayed frame remains.
    let _ = recv_until(&mut peer, QUIET, |_| false).await;

    let mut late = connect(&server).await;
    let (late_client_id, awareness) = awareness_update();
    send(&mut late, DOC_A, awareness).await;
    assert_eq!(
        recv(&mut peer, QUIET).await,
        None,
        "nothing may relay before auth"
    );

    send(&mut late, DOC_A, Body::AuthToken(mint(SECRET))).await;

    let relayed = recv_until(
        &mut peer,
        REPLY,
        |f| matches!(&f.body, Body::Awareness(u) if u.clients.contains_key(&late_client_id)),
    )
    .await;
    assert!(
        relayed.iter().any(
            |f| matches!(&f.body, Body::Awareness(u) if u.clients.contains_key(&late_client_id))
        ),
        "peer must receive the awareness that was queued before the sender's auth; got {relayed:?}"
    );
}

/// Frames buffered under a token that is then rejected must be discarded, and
/// must not resurface if the client later authenticates successfully.
#[tokio::test]
async fn queued_frames_are_dropped_when_auth_fails() {
    let server = start_server().await;
    let mut ws = connect(&server).await;

    send(&mut ws, DOC_A, step1()).await;
    send(&mut ws, DOC_A, Body::AuthToken(mint("wrong-secret"))).await;

    let reply = recv(&mut ws, REPLY)
        .await
        .expect("bad token must be answered");
    assert!(
        is_denied(&reply),
        "bad token must be answered with PermissionDenied, got {:?}",
        reply.body
    );
    assert_eq!(
        recv(&mut ws, QUIET).await,
        None,
        "queued frames must not be applied after a failed auth"
    );

    send(&mut ws, DOC_A, Body::AuthToken(mint(SECRET))).await;
    let frames = recv_until(&mut ws, QUIET, is_step2_for(DOC_A)).await;
    assert!(
        matches!(
            frames.first().map(|f| &f.body),
            Some(Body::Authenticated(_))
        ),
        "a later valid token must still authenticate; got {frames:?}"
    );
    assert!(
        !frames.iter().any(is_step2_for(DOC_A)),
        "frames queued under the failed attempt must have been discarded, but a SyncStep2 arrived: {frames:?}"
    );
}

/// The queue is keyed by document: authenticating one document on a
/// multiplexed socket must not release frames held for another.
#[tokio::test]
async fn queue_is_keyed_per_document() {
    let server = start_server().await;
    let mut ws = connect(&server).await;

    send(&mut ws, DOC_B, step1()).await;
    authenticate(&mut ws, DOC_A).await;
    assert_eq!(
        recv(&mut ws, QUIET).await,
        None,
        "authenticating DOC_A must not release the frame queued for DOC_B"
    );

    send(&mut ws, DOC_B, Body::AuthToken(mint(SECRET))).await;
    let frames = recv_until(&mut ws, REPLY, is_step2_for(DOC_B)).await;
    assert_none_denied(&frames);
    assert!(
        frames.iter().any(is_step2_for(DOC_B)),
        "DOC_B's queued SyncStep1 must be answered once DOC_B authenticates; got {frames:?}"
    );
}

/// An unauthenticated client cannot grow the queue without bound: past the
/// frame cap the server refuses, discards what it held, and stops buffering.
#[tokio::test]
async fn queue_overflow_by_frame_count_is_refused_and_discarded() {
    let server = start_server().await;
    let mut ws = connect(&server).await;

    for _ in 0..PREAUTH_QUEUE_MAX_FRAMES {
        send(&mut ws, DOC_A, step1()).await;
    }
    assert_eq!(
        recv(&mut ws, QUIET).await,
        None,
        "exactly the cap must still be accepted silently"
    );

    send(&mut ws, DOC_A, step1()).await;
    let reply = recv(&mut ws, REPLY)
        .await
        .expect("overflow must be answered");
    assert!(
        is_denied(&reply),
        "the frame past the cap must be refused with PermissionDenied, got {:?}",
        reply.body
    );

    send(&mut ws, DOC_A, Body::AuthToken(mint(SECRET))).await;
    let frames = recv_until(&mut ws, QUIET, is_step2_for(DOC_A)).await;
    assert!(
        matches!(
            frames.first().map(|f| &f.body),
            Some(Body::Authenticated(_))
        ),
        "overflow must not poison a later valid auth; got {frames:?}"
    );
    assert!(
        !frames.iter().any(is_step2_for(DOC_A)),
        "everything buffered before the overflow must have been discarded, but a SyncStep2 arrived: {frames:?}"
    );
}

/// A single oversized frame must trip the byte cap even when the frame cap is
/// nowhere near reached, so a large `Update` cannot be parked pre-auth.
#[tokio::test]
async fn queue_overflow_by_bytes_is_refused() {
    let server = start_server().await;
    let mut ws = connect(&server).await;

    let oversized = vec![0u8; PREAUTH_QUEUE_MAX_BYTES + 1];
    send(&mut ws, DOC_A, Body::Update(oversized)).await;

    let reply = recv(&mut ws, REPLY)
        .await
        .expect("oversized pre-auth frame must be answered");
    assert!(
        is_denied(&reply),
        "an oversized pre-auth frame must be refused with PermissionDenied, got {:?}",
        reply.body
    );
}

/// Behaviour for the normal, fast path is unchanged: the token first, then
/// sync, still yields Authenticated followed by the join handshake.
#[tokio::test]
async fn auth_first_still_works_unchanged() {
    let server = start_server().await;
    let mut ws = connect(&server).await;

    authenticate(&mut ws, DOC_A).await;
    send(&mut ws, DOC_A, step1()).await;
    let frames = recv_until(&mut ws, REPLY, is_step2_for(DOC_A)).await;
    assert_none_denied(&frames);
    assert!(
        frames.iter().any(is_step2_for(DOC_A)),
        "post-auth SyncStep1 must be answered with SyncStep2; got {frames:?}"
    );
}

/// The per-document caps alone would let one socket hold state for an
/// unbounded number of distinct names. Past the per-connection document cap
/// the server must refuse without recording the name at all.
#[tokio::test]
async fn pending_documents_per_connection_are_capped() {
    let server = start_server().await;
    let mut ws = connect(&server).await;

    let names: Vec<String> = (0..=PREAUTH_QUEUE_MAX_DOCS)
        .map(|i| format!("org.rel.{i:08}-0000-0000-0000-000000000000-v0"))
        .collect();
    let (within, past) = names.split_at(PREAUTH_QUEUE_MAX_DOCS);

    for name in within {
        send(&mut ws, name, step1()).await;
    }
    assert_eq!(
        recv(&mut ws, QUIET).await,
        None,
        "up to the cap, distinct documents must each be queued silently"
    );

    let extra = &past[0];
    send(&mut ws, extra, step1()).await;
    let reply = recv(&mut ws, REPLY)
        .await
        .expect("frame past the document cap must be answered");
    assert!(
        reply.name == *extra && is_denied(&reply),
        "the document past the cap must be refused with PermissionDenied, got {reply:?}"
    );

    // The refused name must not have been recorded: authenticating it yields
    // no replay, while a name within the cap still replays its queued frame.
    send(&mut ws, extra, Body::AuthToken(mint(SECRET))).await;
    let frames = recv_until(&mut ws, QUIET, is_step2_for(extra)).await;
    assert!(
        matches!(
            frames.first().map(|f| &f.body),
            Some(Body::Authenticated(_))
        ),
        "the refused document must still be able to authenticate; got {frames:?}"
    );
    assert!(
        !frames.iter().any(is_step2_for(extra)),
        "a frame refused at the document cap must not have been queued: {frames:?}"
    );

    let kept = &within[0];
    send(&mut ws, kept, Body::AuthToken(mint(SECRET))).await;
    let frames = recv_until(&mut ws, REPLY, is_step2_for(kept)).await;
    assert!(
        frames.iter().any(is_step2_for(kept)),
        "documents within the cap keep their queued frames; got {frames:?}"
    );
}

/// A rejected token must not demote a document that already authenticated on
/// this socket: its later frames are applied, not queued.
#[tokio::test]
async fn rejected_token_does_not_revoke_an_authenticated_document() {
    let server = start_server().await;
    let mut ws = connect(&server).await;

    authenticate(&mut ws, DOC_A).await;
    send(&mut ws, DOC_A, step1()).await;
    let _ = recv_until(&mut ws, QUIET, |_| false).await;

    send(&mut ws, DOC_A, Body::AuthToken(mint("wrong-secret"))).await;
    let reply = recv(&mut ws, REPLY)
        .await
        .expect("bad token must be answered");
    assert!(
        is_denied(&reply),
        "expected PermissionDenied, got {:?}",
        reply.body
    );

    send(&mut ws, DOC_A, step1()).await;
    let frames = recv_until(&mut ws, REPLY, is_step2_for(DOC_A)).await;
    assert!(
        frames.iter().any(is_step2_for(DOC_A)),
        "an already-authenticated document must keep applying frames after an unrelated bad token; got {frames:?}"
    );
}
