//! Hocuspocus wire protocol framing over the lib0 codec.
//!
//! Each binary message on the wire is `[varString name][varUint type][payload]`.
//! Payloads for sync/awareness/awareness-query reuse `yrs` types verbatim; the
//! remaining tags (auth, stateless, close, sync-status) are modeled here.

use thiserror::Error;
use yrs::sync::AwarenessUpdate;
use yrs::StateVector;

/// A decoded Hocuspocus frame with its in-band document name.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub name: String,
    pub body: Body,
}

/// Body of a Hocuspocus frame, one variant per `(outer_tag, sub_tag)` pair.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    SyncStep1(StateVector),
    SyncStep2(Vec<u8>),
    Update(Vec<u8>),
    Awareness(AwarenessUpdate),
    AwarenessQuery,
    AuthToken(String),
    Authenticated(String),
    PermissionDenied(String),
    Stateless(String),
    SyncStatus(bool),
    Close,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("buffer truncated")]
    Truncated,
    #[error("invalid utf-8 in string field")]
    Utf8,
    #[error("unknown outer message tag: {0}")]
    UnknownTag(u8),
    #[error("unknown sync sub-tag: {0}")]
    UnknownSyncTag(u8),
    #[error("unknown auth sub-tag: {0}")]
    UnknownAuthTag(u8),
    #[error("malformed payload for tag {tag}: {reason}")]
    Malformed { tag: u8, reason: String },
}

impl Frame {
    pub fn decode(_bytes: &[u8]) -> Result<Frame, ProtocolError> {
        todo!("decode in Phase 3")
    }

    pub fn encode(&self) -> Vec<u8> {
        todo!("encode in Phase 3")
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
