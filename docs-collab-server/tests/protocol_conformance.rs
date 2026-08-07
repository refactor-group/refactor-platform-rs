//! Frozen Hocuspocus wire-protocol conformance tests.
//!
//! Drives `Frame::decode` and `Frame::encode` against committed fixtures
//! produced by `tests/fixtures/capture/capture.mjs`. The fixtures are the
//! source of truth, NOT this file or the harness.

use std::fs;
use std::path::PathBuf;

use docs_collab_server::{Body, Frame, ProtocolError};
use proptest::prelude::*;
use serde::Deserialize;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[derive(Debug, Deserialize)]
struct Manifest {
    fixtures: Vec<FixtureMeta>,
}

#[derive(Debug, Deserialize)]
struct FixtureMeta {
    file: String,
    doc_name: String,
    kind: String,
    #[serde(default)]
    payload_string: Option<String>,
    #[serde(default)]
    payload_bool: Option<bool>,
}

fn load_manifest() -> Manifest {
    let raw = fs::read(fixtures_dir().join("manifest.json")).expect("manifest.json missing");
    serde_json::from_slice(&raw).expect("manifest.json malformed")
}

fn body_kind(b: &Body) -> &'static str {
    match b {
        Body::SyncStep1(_) => "SyncStep1",
        Body::SyncStep2(_) => "SyncStep2",
        Body::Update(_) => "Update",
        Body::Awareness(_) => "Awareness",
        Body::AwarenessQuery => "AwarenessQuery",
        Body::AuthToken(_) => "AuthToken",
        Body::Authenticated(_) => "Authenticated",
        Body::PermissionDenied(_) => "PermissionDenied",
        Body::Stateless(_) => "Stateless",
        Body::SyncStatus(_) => "SyncStatus",
        Body::Close => "Close",
    }
}

#[test]
fn every_fixture_decodes_round_trips_byte_identical() {
    let manifest = load_manifest();
    assert!(!manifest.fixtures.is_empty(), "manifest must list fixtures");
    for meta in &manifest.fixtures {
        let bytes = fs::read(fixtures_dir().join(&meta.file))
            .unwrap_or_else(|e| panic!("read {} failed: {e}", meta.file));

        let frame =
            Frame::decode(&bytes).unwrap_or_else(|e| panic!("decode {} failed: {e:?}", meta.file));
        assert_eq!(
            frame.name, meta.doc_name,
            "doc name mismatch in {}",
            meta.file
        );
        assert_eq!(
            body_kind(&frame.body),
            meta.kind,
            "variant mismatch in {}",
            meta.file
        );

        if let Some(expected) = meta.payload_string.as_deref() {
            let actual = match &frame.body {
                Body::AuthToken(s)
                | Body::Authenticated(s)
                | Body::PermissionDenied(s)
                | Body::Stateless(s) => Some(s.as_str()),
                _ => None,
            };
            assert_eq!(
                actual,
                Some(expected),
                "string payload mismatch in {}",
                meta.file
            );
        }
        if let Some(expected) = meta.payload_bool {
            if let Body::SyncStatus(b) = &frame.body {
                assert_eq!(*b, expected, "sync_status mismatch in {}", meta.file);
            } else {
                panic!("payload_bool only valid on SyncStatus, file {}", meta.file);
            }
        }

        let re = frame.encode();
        assert_eq!(
            re, bytes,
            "encode round-trip not byte-identical for {}",
            meta.file
        );
    }
}

fn doc_name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_.\\-]{1,40}".prop_map(|s| s)
}

fn simple_body_strategy() -> impl Strategy<Value = Body> {
    prop_oneof![
        any::<Vec<u8>>().prop_map(Body::SyncStep2),
        any::<Vec<u8>>().prop_map(Body::Update),
        Just(Body::AwarenessQuery),
        ".*".prop_map(Body::AuthToken),
        ".*".prop_map(Body::Authenticated),
        ".*".prop_map(Body::PermissionDenied),
        ".*".prop_map(Body::Stateless),
        any::<bool>().prop_map(Body::SyncStatus),
        Just(Body::Close),
    ]
}

proptest! {
    #[test]
    fn round_trip_simple_bodies(
        name in doc_name_strategy(),
        body in simple_body_strategy(),
    ) {
        let frame = Frame { name: name.clone(), body: body.clone() };
        let bytes = frame.encode();
        let decoded = Frame::decode(&bytes).expect("decode after encode");
        prop_assert_eq!(decoded.name, name);
        prop_assert_eq!(decoded.body, body);
    }
}

#[test]
fn empty_buffer_is_an_error() {
    assert!(Frame::decode(&[]).is_err());
}

#[test]
fn truncated_after_name_is_an_error() {
    // varString len=1 + 'x', then nothing for the outer tag.
    let bytes = [0x01u8, b'x'];
    assert!(Frame::decode(&bytes).is_err());
}

#[test]
fn unknown_outer_tag_is_rejected() {
    // varString "x" + varUint 99 (unused tag).
    let bytes = [0x01u8, b'x', 99];
    let err = Frame::decode(&bytes).expect_err("must reject unknown tag");
    assert!(
        matches!(err, ProtocolError::UnknownTag(99)),
        "expected UnknownTag(99), got {err:?}"
    );
}

#[test]
fn bad_utf8_in_doc_name_is_an_error() {
    // varString len=1 + invalid utf-8 start byte 0xff.
    let bytes = [0x01u8, 0xff];
    let err = Frame::decode(&bytes).expect_err("must reject bad utf-8");
    assert!(
        matches!(err, ProtocolError::Utf8),
        "expected Utf8, got {err:?}"
    );
}

#[test]
fn unknown_auth_subtag_is_rejected() {
    // varString "x" + outer Auth(2) + sub-tag 99
    let bytes = [0x01u8, b'x', 2, 99];
    assert!(matches!(
        Frame::decode(&bytes),
        Err(ProtocolError::UnknownAuthTag(_) | ProtocolError::Malformed { .. })
    ));
}

#[test]
fn unknown_sync_subtag_is_rejected() {
    // varString "x" + outer Sync(0) + sub-tag 99
    let bytes = [0x01u8, b'x', 0, 99];
    assert!(matches!(
        Frame::decode(&bytes),
        Err(ProtocolError::UnknownSyncTag(_) | ProtocolError::Malformed { .. })
    ));
}
