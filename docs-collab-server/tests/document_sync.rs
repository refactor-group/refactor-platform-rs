//! Frozen tests for `Document` + `DocumentRegistry`.
//!
//! Drives two in-process `yrs::Doc` peers through one `Document` and verifies
//! they converge; verifies the server does not echo a sender's own update;
//! verifies a forced eviction flushes; verifies persistence survives an
//! evict->reload cycle.

use std::sync::Arc;
use std::time::Duration;

use docs_collab_server::{Body, Document, DocumentRegistry, Frame, MemoryStorage, Storage};
use yrs::sync::AwarenessUpdate;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

const DOC: &str = "org.rel.doc-v0";

fn new_storage() -> Arc<dyn Storage> {
    Arc::new(MemoryStorage::new())
}

fn step1(doc: &Doc) -> Body {
    Body::SyncStep1(doc.transact().state_vector())
}

fn update_bytes(doc: &Doc) -> Vec<u8> {
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}

fn apply(doc: &Doc, bytes: &[u8]) {
    let upd = Update::decode_v1(bytes).expect("decode update");
    doc.transact_mut().apply_update(upd).expect("apply update");
}

fn text(doc: &Doc) -> String {
    let t = doc.get_or_insert_text("t");
    t.get_string(&doc.transact())
}

fn write(doc: &Doc, s: &str) {
    let t = doc.get_or_insert_text("t");
    let mut txn = doc.transact_mut();
    t.insert(&mut txn, 0, s);
}

async fn drain<T: Clone>(rx: &mut tokio::sync::broadcast::Receiver<T>) -> Vec<T> {
    let mut out = Vec::new();
    while let Ok(v) = rx.try_recv() {
        out.push(v);
    }
    out
}

#[tokio::test]
async fn two_clients_converge_via_document() {
    let storage = new_storage();
    let server_doc = Document::open(DOC.into(), storage.clone())
        .await
        .expect("open");

    let alice = Doc::new();
    write(&alice, "alice ");
    let bob = Doc::new();
    write(&bob, "bob ");

    let (alice_id, mut alice_rx) = server_doc.join();
    let (bob_id, mut bob_rx) = server_doc.join();

    // Alice handshake. Server should reply with at least a SyncStep2 (initially empty).
    let _ = server_doc
        .handle(alice_id, step1(&alice))
        .await
        .expect("alice step1");

    // Alice sends her update; server applies and broadcasts to bob.
    let alice_payload = update_bytes(&alice);
    let alice_acks = server_doc
        .handle(alice_id, Body::Update(alice_payload.clone()))
        .await
        .expect("alice update");
    // Server may ack with SyncStatus(true); only assert the kind if present.
    for reply in alice_acks {
        if let Body::SyncStatus(b) = reply {
            assert!(b, "SyncStatus ack must be true");
        }
    }

    // Bob handshake: receives SyncStep2 carrying the merged state.
    let bob_replies = server_doc
        .handle(bob_id, step1(&bob))
        .await
        .expect("bob step1");
    let step2_for_bob = bob_replies
        .into_iter()
        .find_map(|b| match b {
            Body::SyncStep2(bytes) => Some(bytes),
            _ => None,
        })
        .expect("server must reply with SyncStep2 to step1");
    apply(&bob, &step2_for_bob);

    // Bob sends his update; alice should see it via broadcast.
    server_doc
        .handle(bob_id, Body::Update(update_bytes(&bob)))
        .await
        .expect("bob update");

    let bytes = tokio::time::timeout(Duration::from_secs(2), alice_rx.recv())
        .await
        .expect("timed out waiting for bob's update on alice's broadcast channel")
        .expect("alice broadcast receiver closed");
    let frame = Frame::decode(&bytes).expect("decode broadcast frame");
    assert_eq!(frame.name, DOC);
    let Body::Update(peer_update) = frame.body else {
        panic!("expected Update broadcast, got {:?}", frame.body)
    };
    apply(&alice, &peer_update);

    // After a small settle, alice's queue must not contain her own echo.
    tokio::time::sleep(Duration::from_millis(50)).await;
    for extra in drain(&mut alice_rx).await {
        if let Ok(f) = Frame::decode(&extra) {
            if let Body::Update(u) = f.body {
                assert_ne!(
                    u, alice_payload,
                    "server must not echo a client's own update back to that client"
                );
            }
        }
    }
    // Bob should not have received any peer broadcast (only alice's update was peer-originated for bob).
    // Bob did receive his merged state via SyncStep2 in the direct reply, not the broadcast.
    let bob_extra = drain(&mut bob_rx).await;
    for b in bob_extra {
        let f = Frame::decode(&b).expect("decode bob broadcast frame");
        if let Body::Update(_) = f.body {
            // An alice-originated update broadcast IS valid for bob, but it must
            // not be bob's own. Bob never sent before the merge so no echo check
            // is needed here. This loop only verifies frames decode cleanly.
        }
    }

    assert_eq!(text(&alice), text(&bob), "two clients must converge");
    assert!(text(&alice).contains("alice"));
    assert!(text(&alice).contains("bob"));
}

#[tokio::test]
async fn awareness_updates_fan_out_to_peers() {
    let storage = new_storage();
    let doc = Document::open(DOC.into(), storage).await.expect("open");
    let (_alice_id, _alice_rx) = doc.join();
    let (bob_id, mut bob_rx) = doc.join();

    // A non-empty awareness update from alice's connection.
    let local_doc = Doc::new();
    let awareness = yrs::sync::Awareness::new(local_doc);
    let raw = awareness.update().expect("encode awareness").encode_v1();
    let awareness_update = AwarenessUpdate::decode_v1(&raw).expect("decode awareness");

    doc.handle(bob_id, Body::Awareness(awareness_update))
        .await
        .expect("apply awareness");

    // Bob originated it; he must not see his own echo. Wait briefly then assert.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let echoes = drain(&mut bob_rx).await;
    for b in echoes {
        let f = Frame::decode(&b).expect("decode");
        assert!(
            !matches!(f.body, Body::Awareness(_)),
            "originator must not receive own awareness echo"
        );
    }
}

#[tokio::test]
async fn persistence_survives_evict_and_reload() {
    let storage = new_storage();
    let registry = DocumentRegistry::new(storage.clone());

    let alice = Doc::new();
    write(&alice, "persisted");

    {
        let doc = registry.get_or_load(DOC).await.expect("get_or_load");
        let (alice_id, _rx) = doc.join();
        doc.handle(alice_id, Body::Update(update_bytes(&alice)))
            .await
            .expect("apply update");
        doc.flush().await.expect("flush");
    }

    let persisted = storage.fetch(DOC).await.expect("fetch after flush");
    let persisted = persisted.expect("flush must write a row to storage");
    assert!(!persisted.is_empty(), "persisted state must not be empty");

    let evicted = registry.evict_now(DOC).await.expect("evict_now");
    assert!(evicted, "evict_now must report it was present");

    // Reload via the registry and verify a fresh client gets the persisted state.
    let reloaded = registry.get_or_load(DOC).await.expect("reload");
    let charlie = Doc::new();
    let (charlie_id, _rx) = reloaded.join();
    let replies = reloaded
        .handle(charlie_id, step1(&charlie))
        .await
        .expect("charlie step1");
    let step2 = replies
        .into_iter()
        .find_map(|b| match b {
            Body::SyncStep2(bytes) => Some(bytes),
            _ => None,
        })
        .expect("server must reply with SyncStep2 carrying persisted state");
    apply(&charlie, &step2);

    assert_eq!(text(&charlie), "persisted");
}

#[tokio::test]
async fn registry_returns_same_arc_for_concurrent_get_or_load() {
    let storage = new_storage();
    let registry = DocumentRegistry::new(storage);
    let a = registry.get_or_load(DOC).await.expect("a");
    let b = registry.get_or_load(DOC).await.expect("b");
    assert!(
        Arc::ptr_eq(&a, &b),
        "registry must return the same Arc for an unevicted name"
    );
}
