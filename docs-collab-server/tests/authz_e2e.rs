//! Frozen end-to-end authorization probe.
//!
//! Ignored by default. Proves that `ws.rs` enforces the JWT scope on a live
//! socket: a cross-scope token and a bad-signature token are both refused at
//! the auth frame and cannot read the document via SyncStep1. The unit tests
//! in `tests/auth.rs` already prove the authenticator logic; this probe proves
//! the wire path actually gates document access.
//!
//! Run against a server started with the matching shared secret:
//!   JWT_SIGNING_KEY=authz-e2e-secret \
//!     DATABASE_URL=... MANAGEMENT_AUTH_KEY=x BIND_ADDR=127.0.0.1:1234 \
//!     cargo run -p docs-collab-server
//!   DOCS_COLLAB_URL=ws://127.0.0.1:1234 JWT_SIGNING_KEY=authz-e2e-secret \
//!     cargo test -p docs-collab-server --test authz_e2e -- --ignored

use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use docs_collab_server::{Body, Frame};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use yrs::StateVector;

// Document name belongs to the relb scope; ALLOW_B covers it, ALLOW_A does not.
// Use a v0 suffix so the name resembles real production document ids.
const DOC_B: &str = "authz-e2e.relb.00000000-0000-0000-0000-000000000000-v0";
const ALLOW_B: &str = "authz-e2e.relb.*";
const ALLOW_A: &str = "authz-e2e.rela.*";

const APP_ID: &str = "tiptap_app_id_value";

// Mirrors `domain/src/jwt/claims.rs` so the token shape matches what the real
// backend mints, including the `aud` claim the server tolerates.
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

fn mint(secret: &str, allowed_prefix: &str, sub: &str, exp_offset_secs: i64) -> String {
    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        exp: (Utc::now() + ChronoDuration::seconds(exp_offset_secs)).timestamp() as usize,
        iat: now,
        ndf: now,
        iss: "https://refactorcoach.com".into(),
        sub: sub.into(),
        aud: APP_ID.into(),
        allowed_document_names: vec![allowed_prefix.to_string()],
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("mint test token")
}

fn server_url() -> String {
    std::env::var("DOCS_COLLAB_URL")
        .expect("set DOCS_COLLAB_URL=ws://127.0.0.1:1234 to run authz_e2e")
}

fn signing_key() -> String {
    std::env::var("JWT_SIGNING_KEY")
        .expect("set JWT_SIGNING_KEY to the secret the server under test was started with")
}

type Ws = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect() -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(server_url())
        .await
        .expect("connect to server");
    ws
}

async fn send_frame(ws: &mut Ws, frame: Frame) {
    ws.send(Message::Binary(frame.encode()))
        .await
        .expect("send frame");
}

// Read the next binary frame, decoding the protocol body. Panics on text/close
// or read error so failures point at the protocol layer rather than transport.
async fn next_body(ws: &mut Ws, wait: Duration) -> Body {
    let msg = timeout(wait, ws.next())
        .await
        .expect("timeout waiting for reply")
        .expect("server hung up")
        .expect("ws error");
    let Message::Binary(bytes) = msg else {
        panic!("expected binary frame, got {msg:?}");
    };
    Frame::decode(&bytes).expect("decode reply").body
}

// Control: a correctly-scoped, correctly-signed token authenticates. This
// guards the rejection cases below from a false sense of security caused by
// a broken harness (e.g. wrong URL, wrong signing key).
#[ignore]
#[tokio::test]
async fn matched_scope_authenticates() {
    let token = mint(&signing_key(), ALLOW_B, DOC_B, 3600);
    let mut ws = connect().await;
    send_frame(
        &mut ws,
        Frame {
            name: DOC_B.into(),
            body: Body::AuthToken(token),
        },
    )
    .await;

    let body = next_body(&mut ws, Duration::from_secs(2)).await;
    assert!(
        matches!(body, Body::Authenticated(_)),
        "control: matched scope must Authenticate, got {body:?}"
    );
}

// Token signed correctly but scoped to a different relationship. The server
// must refuse and refuse to surface document state on a follow-up SyncStep1.
#[ignore]
#[tokio::test]
async fn cross_scope_is_denied_and_cannot_sync() {
    let token = mint(&signing_key(), ALLOW_A, "authz-e2e.rela.subject-v0", 3600);
    let mut ws = connect().await;
    send_frame(
        &mut ws,
        Frame {
            name: DOC_B.into(),
            body: Body::AuthToken(token),
        },
    )
    .await;

    let body = next_body(&mut ws, Duration::from_secs(2)).await;
    assert!(
        matches!(body, Body::PermissionDenied(_)),
        "cross-scope must be denied, got {body:?}"
    );

    // Even after denial, a forbidden client must never receive SyncStep2 for
    // the protected doc. Send SyncStep1, then drain frames until timeout.
    send_frame(
        &mut ws,
        Frame {
            name: DOC_B.into(),
            body: Body::SyncStep1(StateVector::default()),
        },
    )
    .await;
    assert_no_sync_step2(&mut ws, Duration::from_millis(1000)).await;
}

// Token with a valid scope on paper but signed with the wrong secret. Since
// signature verification precedes scope check, the server must refuse without
// trusting any claim, including the document name.
#[ignore]
#[tokio::test]
async fn bad_signature_is_denied_and_cannot_sync() {
    let token = mint("a-different-secret", ALLOW_B, DOC_B, 3600);
    let mut ws = connect().await;
    send_frame(
        &mut ws,
        Frame {
            name: DOC_B.into(),
            body: Body::AuthToken(token),
        },
    )
    .await;

    let body = next_body(&mut ws, Duration::from_secs(2)).await;
    assert!(
        matches!(body, Body::PermissionDenied(_)),
        "bad signature must be denied, got {body:?}"
    );

    send_frame(
        &mut ws,
        Frame {
            name: DOC_B.into(),
            body: Body::SyncStep1(StateVector::default()),
        },
    )
    .await;
    assert_no_sync_step2(&mut ws, Duration::from_millis(1000)).await;
}

// Drain frames until the socket goes quiet (`next` times out) or the server
// closes. Any SyncStep2 observed in that window is a leak of document state
// to a client the server already rejected, and fails the test.
async fn assert_no_sync_step2(ws: &mut Ws, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return;
        }
        let next = timeout(remaining, ws.next()).await;
        let msg = match next {
            Err(_) => return,           // window elapsed with no SyncStep2: pass
            Ok(None) => return,         // server closed: pass
            Ok(Some(Err(_))) => return, // ws error / close frame: pass
            Ok(Some(Ok(m))) => m,
        };
        let Message::Binary(bytes) = msg else {
            continue;
        };
        if let Ok(frame) = Frame::decode(&bytes) {
            assert!(
                !matches!(frame.body, Body::SyncStep2(_)),
                "forbidden client received SyncStep2 for {DOC_B}: protocol leaked doc state"
            );
        }
    }
}
