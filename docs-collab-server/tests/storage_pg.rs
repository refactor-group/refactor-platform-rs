//! Frozen PostgresStorage integration tests.
//!
//! Ignored by default. Run with a reachable Postgres and the env var:
//!   DATABASE_URL=postgres://refactor:password@localhost:5432/refactor \
//!     cargo test -p docs-collab-server --test storage_pg -- --ignored
//!
//! The schema name is fixed (`docs_collab_test`) and created if absent by
//! `PostgresStorage::connect`. Tests namespace document names per-run to avoid
//! cross-test interference.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use docs_collab_server::{PostgresStorage, Storage};

const TEST_SCHEMA: &str = "docs_collab_test";

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for storage_pg tests")
}

async fn fresh_storage() -> PostgresStorage {
    PostgresStorage::connect(&database_url(), TEST_SCHEMA)
        .await
        .expect("connect to test Postgres")
}

fn uniq() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{ts}-{n}")
}

#[ignore]
#[tokio::test]
async fn store_fetch_round_trip() {
    let s = fresh_storage().await;
    let name = format!("rt-{}", uniq());
    let bytes: Vec<u8> = (0..256).map(|i| i as u8).collect();
    s.store(&name, &bytes).await.expect("store");
    let got = s.fetch(&name).await.expect("fetch");
    assert_eq!(got.as_deref(), Some(bytes.as_slice()));
    s.delete(&name).await.expect("delete");
}

#[ignore]
#[tokio::test]
async fn fetch_missing_returns_none() {
    let s = fresh_storage().await;
    let got = s
        .fetch(&format!("missing-{}", uniq()))
        .await
        .expect("fetch");
    assert!(got.is_none());
}

#[ignore]
#[tokio::test]
async fn store_is_idempotent_upsert() {
    let s = fresh_storage().await;
    let name = format!("upsert-{}", uniq());
    s.store(&name, b"first").await.expect("store");
    s.store(&name, b"second").await.expect("upsert");
    assert_eq!(
        s.fetch(&name).await.expect("fetch").as_deref(),
        Some(&b"second"[..])
    );
    s.delete(&name).await.expect("delete");
}

#[ignore]
#[tokio::test]
async fn delete_missing_is_ok() {
    let s = fresh_storage().await;
    s.delete(&format!("never-existed-{}", uniq()))
        .await
        .expect("delete of missing row must not error");
}
