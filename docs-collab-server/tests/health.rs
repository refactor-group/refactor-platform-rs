//! The readiness check answers 200 only while storage answers.
//!
//! These tests boot the real router on an ephemeral port and send a plain
//! HTTP request over a real socket.

use std::future::IntoFuture;
use std::sync::Arc;

use async_trait::async_trait;
use docs_collab_server::storage::StorageError;
use docs_collab_server::{
    build_router, AppState, DocumentRegistry, JwtAuthenticator, MemoryStorage, Storage,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// Storage whose database is unreachable.
struct UnreachableStorage;

#[async_trait]
impl Storage for UnreachableStorage {
    async fn fetch(&self, _name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Err(StorageError::Backend("unreachable".into()))
    }

    async fn store(&self, _name: &str, _state: &[u8]) -> Result<(), StorageError> {
        Err(StorageError::Backend("unreachable".into()))
    }

    async fn delete(&self, _name: &str) -> Result<(), StorageError> {
        Err(StorageError::Backend("unreachable".into()))
    }

    async fn ping(&self) -> Result<(), StorageError> {
        Err(StorageError::Backend("unreachable".into()))
    }
}

/// Status line of `GET path` against a server backed by `storage`.
async fn status_line(storage: Arc<dyn Storage>, path: &str) -> String {
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let state = AppState {
        registry: DocumentRegistry::new(storage.clone()),
        storage,
        authenticator: Arc::new(JwtAuthenticator::new(b"unused-in-these-tests")),
        management_auth_key: Arc::from("unused-in-these-tests"),
        shutdown: shutdown_rx,
    };
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(axum::serve(listener, build_router(state)).into_future());

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .expect("send request");
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.expect("read reply");
    reply.lines().next().unwrap_or_default().to_string()
}

#[tokio::test]
async fn ready_when_storage_answers() {
    assert_eq!(
        status_line(Arc::new(MemoryStorage::new()), "/health/ready").await,
        "HTTP/1.1 200 OK"
    );
}

#[tokio::test]
async fn not_ready_when_storage_is_unreachable() {
    assert_eq!(
        status_line(Arc::new(UnreachableStorage), "/health/ready").await,
        "HTTP/1.1 503 Service Unavailable"
    );
}

#[tokio::test]
async fn liveness_ignores_storage() {
    assert_eq!(
        status_line(Arc::new(UnreachableStorage), "/health").await,
        "HTTP/1.1 200 OK"
    );
}
