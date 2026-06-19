//! Hocuspocus wire protocol framing over the lib0 codec.
//!
//! Each binary message on the wire is `[varString name][varUint type][payload]`.
//! Payloads for sync/awareness/awareness-query reuse `yrs` types verbatim; the
//! remaining tags (auth, stateless, close, sync-status) are modeled here.

use thiserror::Error;
use yrs::encoding::read::{Cursor, Read};
use yrs::encoding::write::Write;
use yrs::sync::AwarenessUpdate;
use yrs::updates::decoder::{Decode, Decoder, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
use yrs::StateVector;

// Outer message tags. Mirrors @hocuspocus/common.
const TAG_SYNC: u8 = 0;
const TAG_AWARENESS: u8 = 1;
const TAG_AUTH: u8 = 2;
const TAG_QUERY_AWARENESS: u8 = 3;
const TAG_STATELESS: u8 = 5;
const TAG_CLOSE: u8 = 7;
const TAG_SYNC_STATUS: u8 = 8;

// Sync sub-tags. Mirrors y-protocols sync.
const SYNC_STEP1: u8 = 0;
const SYNC_STEP2: u8 = 1;
const SYNC_UPDATE: u8 = 2;

// Auth sub-tags. Mirrors @hocuspocus.
const AUTH_TOKEN: u8 = 0;
const AUTH_PERMISSION_DENIED: u8 = 1;
const AUTH_AUTHENTICATED: u8 = 2;

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

// Read a varUint-length-prefixed byte buffer, then validate utf-8 ourselves
// so we can distinguish Truncated (length/read failure) from Utf8 (decode failure).
fn read_var_string<D: Decoder>(dec: &mut D) -> Result<String, ProtocolError> {
    let bytes = dec.read_buf().map_err(|_| ProtocolError::Truncated)?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| ProtocolError::Utf8)
}

fn read_var_buf<D: Decoder>(dec: &mut D) -> Result<Vec<u8>, ProtocolError> {
    dec.read_buf()
        .map(|b| b.to_vec())
        .map_err(|_| ProtocolError::Truncated)
}

fn read_u8_var<D: Decoder>(dec: &mut D) -> Result<u8, ProtocolError> {
    dec.read_var::<u8>().map_err(|_| ProtocolError::Truncated)
}

impl Frame {
    pub fn decode(bytes: &[u8]) -> Result<Frame, ProtocolError> {
        let mut dec = DecoderV1::new(Cursor::new(bytes));
        let name = read_var_string(&mut dec)?;
        let outer = read_u8_var(&mut dec)?;

        let body = match outer {
            TAG_SYNC => decode_sync(&mut dec)?,
            TAG_AWARENESS => {
                let payload = read_var_buf(&mut dec)?;
                AwarenessUpdate::decode_v1(&payload)
                    .map(Body::Awareness)
                    .map_err(|e| ProtocolError::Malformed {
                        tag: TAG_AWARENESS,
                        reason: format!("awareness: {e}"),
                    })?
            }
            TAG_AUTH => decode_auth(&mut dec)?,
            TAG_QUERY_AWARENESS => Body::AwarenessQuery,
            TAG_STATELESS => Body::Stateless(read_var_string(&mut dec)?),
            TAG_CLOSE => Body::Close,
            TAG_SYNC_STATUS => dec
                .read_var::<i64>()
                .map(|v| Body::SyncStatus(v == 1))
                .map_err(|_| ProtocolError::Truncated)?,
            other => return Err(ProtocolError::UnknownTag(other)),
        };

        Ok(Frame { name, body })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut enc = EncoderV1::new();
        enc.write_string(&self.name);
        match &self.body {
            Body::SyncStep1(sv) => {
                enc.write_var(TAG_SYNC);
                enc.write_var(SYNC_STEP1);
                enc.write_buf(sv.encode_v1());
            }
            Body::SyncStep2(bytes) => {
                enc.write_var(TAG_SYNC);
                enc.write_var(SYNC_STEP2);
                enc.write_buf(bytes);
            }
            Body::Update(bytes) => {
                enc.write_var(TAG_SYNC);
                enc.write_var(SYNC_UPDATE);
                enc.write_buf(bytes);
            }
            Body::Awareness(upd) => {
                enc.write_var(TAG_AWARENESS);
                enc.write_buf(upd.encode_v1());
            }
            Body::AwarenessQuery => {
                enc.write_var(TAG_QUERY_AWARENESS);
            }
            Body::AuthToken(s) => {
                enc.write_var(TAG_AUTH);
                enc.write_var(AUTH_TOKEN);
                enc.write_string(s);
            }
            Body::Authenticated(s) => {
                enc.write_var(TAG_AUTH);
                enc.write_var(AUTH_AUTHENTICATED);
                enc.write_string(s);
            }
            Body::PermissionDenied(s) => {
                enc.write_var(TAG_AUTH);
                enc.write_var(AUTH_PERMISSION_DENIED);
                enc.write_string(s);
            }
            Body::Stateless(s) => {
                enc.write_var(TAG_STATELESS);
                enc.write_string(s);
            }
            Body::SyncStatus(b) => {
                enc.write_var(TAG_SYNC_STATUS);
                enc.write_var(if *b { 1i64 } else { 0i64 });
            }
            Body::Close => {
                enc.write_var(TAG_CLOSE);
            }
        }
        enc.to_vec()
    }
}

// Dispatch the Sync sub-tag BEFORE reading the payload. This lets a known
// outer tag with an unknown sub-tag surface as `UnknownSyncTag`, even when
// the buffer has nothing after the sub-tag.
fn decode_sync<D: Decoder>(dec: &mut D) -> Result<Body, ProtocolError> {
    let sub = read_u8_var(dec)?;
    match sub {
        SYNC_STEP1 => {
            let payload = read_var_buf(dec)?;
            StateVector::decode_v1(&payload)
                .map(Body::SyncStep1)
                .map_err(|e| ProtocolError::Malformed {
                    tag: TAG_SYNC,
                    reason: format!("state vector: {e}"),
                })
        }
        SYNC_STEP2 => Ok(Body::SyncStep2(read_var_buf(dec)?)),
        SYNC_UPDATE => Ok(Body::Update(read_var_buf(dec)?)),
        other => Err(ProtocolError::UnknownSyncTag(other)),
    }
}

// Dispatch the Auth sub-tag BEFORE reading the string. Same rationale: an
// unknown sub-tag must reject as `UnknownAuthTag`, not as `Truncated`.
fn decode_auth<D: Decoder>(dec: &mut D) -> Result<Body, ProtocolError> {
    let sub = read_u8_var(dec)?;
    match sub {
        AUTH_TOKEN => Ok(Body::AuthToken(read_var_string(dec)?)),
        AUTH_PERMISSION_DENIED => Ok(Body::PermissionDenied(read_var_string(dec)?)),
        AUTH_AUTHENTICATED => Ok(Body::Authenticated(read_var_string(dec)?)),
        other => Err(ProtocolError::UnknownAuthTag(other)),
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
