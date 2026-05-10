//! TipTap Cloud metrics gateway.
//!
//! Exposes read-only methods over TipTap's REST surface for platform-level
//! observability: total document counts, per-document listings, and the
//! ingredients for abandoned document detection.
//!
//! Pattern source: sibling `mailersend.rs` and `tiptap.rs` are the canonical
//! gateway shapes used in this crate. We follow their auth-header,
//! error-mapping, and inline-test conventions.

// `//!` is an *inner* doc comment - it documents the containing module/file.
// `///` is an *outer* doc comment - it documents the next item below it.

// Conventions: every gateway module starts with a `//!` block describing
// scope, source-of-truth API, and the existing pattern it follows.

// Imports follow a strict three-block order from `.claude/coding-standards.md`:
//  1. `std::*`         (none needed yet - added in Session 2)
//  2. external crates  (alphabetical)
//  3. `crate::*`       (alphabetical)
// Each block is separated by a single blank line. Clippy will not flag
// disorder, but reviewers will.
use serde::Deserialize;

#[allow(unused_imports)]
use service::config::Config;

#[allow(unused_imports)]
use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};

// ------------------------------------------------
// Client
// ------------------------------------------------

// `pub(crate)` = visible inside the `domain` crate, invisible to `web/` and `entity_api/`.
// This is the encapsulation boundary CLAUDE.md enforces:
// gateways are a domain-layer implementation detail. If `web/` ever needs
// a gateway type, that's a smell - domain should expose its own type instead.
#[allow(dead_code)]
pub(crate) struct Client {
    // `reqwest::Client` is internally `Arc<...>`. Cloning is a cheap atomic
    // refcount bump that shares one connection pool process-wide.
    // The recommended pattern is "one Client, clone freely." We hold by
    // value here; callers can hold by value or behind their own `Arc` if they
    // share across many tasks.
    client: reqwest::Client,
    // The resolved base URL (e.g. "https://<app_id>.collab.tiptap.cloud").
    // Storing it once means each method `format!`s a path against it
    // rather than re-reading config on every call. Fewer allocations, fewer
    // `Option<String>` unwraps later.
    base_url: String,
}

// -------------------------------------------------
// Response types
// -------------------------------------------------
//
// Two design principles for these structs:
//
//  1.  Match the wire format with `#[serde(rename_all = "camelCase")]`. TipTap's
//      JSON uses camelCase; Rust convention is snake_case. The attribute does
//      the rename on every field automatically - much cleaner than per-field
//      `#[serde(rename = "totalDocuments")]`.
//
//  2.  Be tolerant of missing/unknown fields. We deliberately do NOT use
//      `#[serde(deny_unknown_fields)]` - TipTap is an external API we don't
//      own, and additive changes shouldn't 500 our endpoint. Per-field
//      `#[serde(default)]` lets a missing key parse as `Default::default()`
//      rather than failing the whole response.

/// Global TipTap statistics returned by `GET /api/statistics`.
///
/// One-shot summary endpoint, no pagination. We pull only the fields we
/// surface; the rest of TipTap's response (e.g. `openDocuments`,
/// `connectionsPerDocument`) is silently ignored thanks to default serde
/// tolerance.
// `Debug` for log lines. `Clone` so the value can be passed by value cheaply.
// `Deserialize` for JSON parsing - and *only* `Deserialize`, no `Serialize`,
// because we never send this struct on the wire (it's an API response).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
// `rename_all` on the struct applies the rename to *every* field. So
// `total_documents` here matches the wire-format `totalDocuments`.
#[serde(rename_all = "camelCase")]
pub(crate) struct Statistics {
    // `u64` rather than `usize`: counts crossing a JSON boundary should be
    // platform-independent. `usize` varies by target architecture; `u64`
    // doesn't.
    pub(crate) total_documents: u64,

    // `#[serde(default)]` = if `currentLoadedDocumentsCount` is missing in
    // the response, parse as `0` (the `Default` for `u64`). This keeps us
    // forward-compatible if TipTap drops a non-essential field - the call
    // still succeeds, the metric just shows 0.
    #[serde(default)]
    pub(crate) current_loaded_documents_count: u64,

    // We don't surface `version` to admins, but capturing it lets us include
    // it in log lines for diagnostics - useful when TipTap server-side
    // schema drifts and we need to correlate with their release notes.
    #[serde(default)]
    pub(crate) version: String,
}

/// A single TipTap document as returned by `GET /api/documents`.
///
/// IMPORTANT - TipTap's documents-list response shape is loosely documented.
/// The `name` field IS guaranteed (it's the document identifier; in our
/// product it equals a coaching_session UUID via
/// `coaching_sessions.collab_document_name`). Other fields here are
/// best-effort - verify against a live response on first deploy and pin
/// the struct to whatever ships.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Document {
    // The document identifier. Maps 1:1 to a coaching session UUID via
    // `coaching_sessions.collab_document_name`. This is the join column
    // we'll use in Sessions 5/6 for per-org aggregation and abandoned-doc
    // detection. Keep it `String` (not `Uuid`) - TipTap treats it as
    // opaque, and so should we until we explicitly parse for our DB query.
    pub(crate) name: String,

    // Approximate byte size of the doc payload. Field name is a best-guess
    // based on TipTap conventions - confirm against a live response. We
    // default to 0 so a missing key produces an undercount rather than a panic.
    // Undercounts are easy to diagnose; panics in admin endpoints are not.
    #[serde(default)]
    pub(crate) size: u64,

    // Server-side archive flag. Defaulting to `false` matches TipTap's
    // semantics: only explicitly archived docs are flagged.
    #[serde(default)]
    pub(crate) archived: bool,
}

// We'll need a paginated response wrapper in Session 4. Add it now because
// it's a "type that exists," not behavior.
//
// TipTap returns documents as a JSON array at the top level. If on first
// probe the response is wrapped (e.g. `{ "data": [...] }`), adjust this
// alias to a struct. Defensive default: alias the array directly and let
// integration tests catch any wrapper drift.
#[allow(dead_code)]
pub(crate) type DocumentsPage = Vec<Document>;
