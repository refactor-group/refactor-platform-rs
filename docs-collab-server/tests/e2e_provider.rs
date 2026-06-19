//! Frozen end-to-end protocol test.
//!
//! Ignored by default. The intent is to drive a running docs-collab-server
//! over a real WebSocket and verify the wire protocol round-trips with a real
//! client. Two viable harnesses:
//!
//! 1. (Preferred per the plan) Spawn `@hocuspocus/provider` 2.15.3 from a Node
//!    child process and use it as the client. Skipped here because the server
//!    bootstrap (`ws.rs`/`main.rs`) lands in Phase 7; until then, this test
//!    has nothing to connect to in-process.
//! 2. (Fallback) Replay the committed wire fixtures over a raw
//!    `tokio-tungstenite` socket against an externally-running server.
//!
//! What we ship in Phase 2 is the fallback shape: it compiles, is `#[ignore]`d
//! by default, and documents the required invariant. Once Phase 7 exposes a
//! public `serve` (or equivalent) so the test can bind an ephemeral port
//! in-process, the body below should be lifted from `DOCS_COLLAB_URL` to that
//! binding.

use std::time::Duration;

use docs_collab_server::{Body, Frame};
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

fn server_url() -> Option<String> {
    std::env::var("DOCS_COLLAB_URL").ok()
}

#[ignore]
#[tokio::test]
async fn handshake_byte_replay_against_running_server() {
    let url = server_url().expect("set DOCS_COLLAB_URL=ws://127.0.0.1:1234 to run");
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("connect to server");

    // Auth/Token frame from committed fixture (the real provider's bytes).
    let auth_bytes = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/auth_token.bin"),
    )
    .expect("auth_token.bin fixture");
    ws.send(Message::Binary(auth_bytes))
        .await
        .expect("send auth");

    // Server is expected to reply with Auth/Authenticated within a short window.
    let reply = timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("timeout waiting for auth reply")
        .expect("server hung up")
        .expect("ws error");

    let Message::Binary(bytes) = reply else {
        panic!("expected binary auth reply, got {reply:?}");
    };
    let frame = Frame::decode(&bytes).expect("decode auth reply");
    assert!(
        matches!(
            frame.body,
            Body::Authenticated(_) | Body::PermissionDenied(_)
        ),
        "first server reply must be an Auth message, got {:?}",
        frame.body
    );
}
