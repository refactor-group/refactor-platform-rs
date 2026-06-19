//! Management REST API for collaborative documents.
//!
//! Two operations: idempotent create (seeds an empty Yjs state for the name
//! when absent) and delete (no-op when absent). Both are gated by a verbatim
//! shared-secret compare against the `Authorization` header, no `Bearer `
//! prefix. The `?format=json` query string is accepted and ignored.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use tracing::warn;
use yrs::{ReadTxn, StateVector, Transact};

use crate::ws::AppState;

/// Reject the request when the `Authorization` header does not match the
/// configured management secret byte-for-byte. Constant-time compare is not
/// used here intentionally: this endpoint is reached over an authenticated
/// internal gateway, and the secret is server-config (not user-derived).
fn check_auth(headers: &HeaderMap, expected: &str) -> Result<(), StatusCode> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .filter(|provided| *provided == expected)
        .map(|_| ())
        .ok_or(StatusCode::UNAUTHORIZED)
}

/// Encode an empty Yjs `Doc` as a v1 update so subsequent loads hydrate
/// cleanly. This matches what the document layer expects from `Storage::fetch`.
fn empty_yjs_seed() -> Vec<u8> {
    // Bind both so reverse drop order frees the txn before the doc, satisfying
    // the txn's borrow on `doc` across the encode call.
    let doc = yrs::Doc::new();
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&StateVector::default())
}

pub async fn create_document(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    check_auth(&headers, &state.management_auth_key)?;

    // Fetch-then-seed so an existing document is not clobbered by the seed
    // bytes. Two concurrent POSTs converge on the same empty seed if both win
    // the race, since the seed is deterministic.
    match state.storage.fetch(&name).await {
        Ok(Some(_)) => Ok(StatusCode::OK),
        Ok(None) => match state.storage.store(&name, &empty_yjs_seed()).await {
            Ok(_) => Ok(StatusCode::CREATED),
            Err(e) => {
                warn!(name = %name, error = %e, "rest: seed store failed");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        },
        Err(e) => {
            warn!(name = %name, error = %e, "rest: existence check failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn delete_document(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    check_auth(&headers, &state.management_auth_key)?;

    // Storage::delete treats a missing row as `Ok`, so this endpoint is
    // idempotent. A currently-live `Document` in the registry will still hold
    // an in-memory copy that its persist task may re-write; callers are
    // expected to delete only documents that are not actively being edited.
    state.storage.delete(&name).await.map_or_else(
        |e| {
            warn!(name = %name, error = %e, "rest: delete failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        },
        |_| Ok(StatusCode::NO_CONTENT),
    )
}
