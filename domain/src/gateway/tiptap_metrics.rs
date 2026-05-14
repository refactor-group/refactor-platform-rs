//! TipTap Cloud metrics gateway.
//!
//! Read-only methods over TipTap's REST surface: total doc counts, per-doc
//! listings, and ingredients for abandoned-doc detection.
//!
//! Pattern source: `mailersend.rs` + `tiptap.rs`. Match their auth-header,
//! error-mapping, and inline-test conventions.

// Doc comments: `//!` is *inner* (documents the enclosing module/file);
// `///` is *outer* (documents the next item).

// Import order: std → external crates → crate::*, blank line between blocks.
// `log`::*` glob brings warn!/info!/debug!/error! into scope.
use std::time::Duration;

use log::*;
use serde::Deserialize;

use service::config::Config;

#[allow(unused_imports)]
use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};

// Bounded waits per call. `Duration` is the canonical type at API boundaries
// never raw `u64` seconds. The sibling `tiptap.rs` sets no timeout (a known
// foutgun;) admin endpoints need bounded budgets so a dead upstream can't
// hold an Axum worker. `connect_timeout` covers DNS+TCP+TLS only; setting it
// short means we fail fast instead of burning the full 40s on a dead SYN.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

// TipTap's documented default; max isn't published. 100 keeps us well clear
// of the rate limit (100 req / 5s per IP) and within sane payload sizes.
const PAGE_SIZE: u32 = 100;

// Safety cap. At 100 docs/page this allows 1M docs before we bail. If we
// ever hit this in practice something is wrong upstream - log + return what
// we have rather than OOM the worker.
const MAX_PAGES: u32 = 10_000;

// -----------------------------------------------------------------------------
// Client
// -----------------------------------------------------------------------------

// `pub(crate)`: visible inside `domain`, hidden from `web/` and `entity_api/`.
// Gateways are a domain-layer implementation detail (CLAUDE.md rule).
#[allow(dead_code)]
pub(crate) struct Client {
    // `reqwest::Client` is internally `Arc<...>`: cloning is a cheap atomic
    // refcount bump that shares one connection pool. One per process; clone freely.
    client: reqwest::Client,
    // Resolved base URL (e.g. "https://<app_id>.collab.tiptap.cloud"). Stored
    // once so methods just `format!` paths against it.
    base_url: String,
}

impl Client {
    /// Construct a TipTap metrics client from app config.
    ///
    /// Returns `InternalErrorKind::Config` if `tiptap_url` or `tiptap_auth_key`
    /// is missing - operator-visible misconfiguration, not transient failure
    //
    // `async fn` matches sibling gateway signatures even though nothing here
    // awaits - keep the API stable if a future impl needs to.
    pub(crate) async fn new(config: &Config) -> Result<Self, Error> {
        // `?` propogates `domain::Error` unchanged - types match exactly.
        let client = build_client(config).await?;

        // `ok_or_else` (not `ok_or`): closure runs only on `None, deferring
        // the `Error` allocation. Clippy's `or_fun_call` flags the eager form.
        let base_url = config.tiptap_url().ok_or_else(|| {
            // `warn!` captures file:line automatically.
            warn!("TipTap URL missing from config (metrics gateway init)");
            Error {
                // Missing input, not a wrapped downstream failure -> no `source`.
                source: None,
                // `Config` variant = "operator forgot TIPTAP_URL"; maps to HTTP 500
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
            }
        })?;

        // Field-init shorthand: `Self { client: client, base_url: base_url }`.
        Ok(Self { client, base_url })
    }

    /// Fetch global TipTap statistics: Get/api/statistics.
    ///
    /// One-shot summary; no pagination. Upstream HTTP and deserialize
    /// failures oboth map to `External::Network` - TipTap drift is an
    /// external concern, not our bug.
    #[allow(dead_code)]
    pub(crate) async fn fetch_statistics(&self) -> Result<Statistics, Error> {
        // `format!` against the stored base. Hardcoded path - TipTap's
        // metrics endpoint is well-defined
        let url = format!("{}/api/statistics", self.base_url);

        // GET with default headers (auth attached in build client).
        // `map_err` over `?` here because reqwest::Error -> External::Network
        // is the default `From` impl, but we want to log the URL too.
        let response = self.client.get(&url).send().await.map_err(|e| {
            warn!("Failed to fetch TipTap statistics from {url}: {e:?}");
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        // Check status BEFORE deserializing - an error body won't shape-match
        // `Statistics`. Reqwest alternative: `response.error_for_status()`.
        // We follow `tiptap.rs`'s manual style for consistency.
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("TipTap /api/statistics returned {status}: {body}");
            return Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            });
        }

        // `.json::<T>()` reads body + parses as JSON into `T`. Deserialize
        // failures = TipTap schema drift = External::Network.
        response.json::<Statistics>().await.map_err(|e| {
            warn!("Failed to deserialize TipTap statistics: {e:?}");
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })
    }

    /// Fetch a single page from `GET /api/documents?skip&take`.
    ///
    /// Private helper - the public surface is `list_all_documents`. Same
    /// error-mapping pattern as `fetch_statistics`: HTTP and deserialize
    /// failures both become `External::Network`.
    async fn fetch_documents_page(&self, skip: u32, take: u32) -> Result<DocumentsPage, Error> {
        let url = format!("{}/api/documents", self.base_url);

        // `.query(&[(...)])` is reqwest's typed query-builder. Values are
        // serialized via Display; integers stringify safely. Cleaner than
        // hand-formatting `?skip=X&take=Y` and avoids URL-encoding bugs.
        let response = self
            .client
            .get(&url)
            .query(&[("skip", skip), ("take", take)])
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to fetch TipTap documents page (skip={skip}): {e:?}");
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("TipTap /api/documents/returned {status} at skip={skip}: {body}");
            return Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            });
        }

        response.json::<DocumentsPage>().await.map_err(|e| {
            warn!("Failed to deserialize TipTap documents page: {e:?}");
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })
    }

    /// Fetch every TipTap document via offset pagination
    ///
    /// CAVEAT: offset pagination is racy under concurrent writes - a doc
    /// inserted at offset N mid-walk can be missed or double-counted
    /// Acceptable for admin observability (eventual consistency is fine)
    /// a stricture consumer would snapshot a timestamp and filter results
    pub(crate) async fn list_all_documents(&self) -> Result<Vec<Document>, Error> {
        let mut all: Vec<Document> = Vec::new();
        let mut skip: u32 = 0;

        // Cap iterations to avoid infinite loop if TipTap misbehaves
        // `0..MAX_PAGES` is exclusive so we stop at MAX_PAGES iterations.
        for _ in 0..MAX_PAGES {
            let page = self.fetch_documents_page(skip, PAGE_SIZE).await?;
            let page_len = page.len() as u32;

            // Move the page into `all` (no clone - `extend` consumes).
            all.extend(page);

            // Short page = end of data. TipTap has no `total` field, so this
            // is the only termination signal we have. Equality check (not <)
            // because exactly-PAGE_SIZE means "more pages possible."
            if page_len < PAGE_SIZE {
                return Ok(all);
            }

            // `saturating_add` defends against u32 overflow ( would take 4B+
            // docs to hit, but it's free defensive coding).
            skip = skip.saturating_add(PAGE_SIZE);
        }

        // Hit the safety cap. Log and return what we collected - partial
        // results are better than an error here because admins still get
        // useful aggregate numbers
        warn!(
            "TipTap document pagination hit MAX_PAGES={MAX_PAGES} cap; \
            returning {} partial results",
            all.len()
        );
        Ok(all)
    }
}

// -----------------------------------------------------------------------------
// Response types
// -----------------------------------------------------------------------------
//
// 1. `#[serde(rename_all = "camelCase")]` maps wire camelCase → snake_case fields.
// 2. NO `deny_unknown_fields` — TipTap is external; additive changes shouldn't
//    500 us. `#[serde(default)]` on optional fields parses missing keys as
//    `Default::default()`.

/// Global TipTap statistics returned by `GET /api/statistics`.
/// One-shot summary, no pagination. Unsurfaced fields are silently ignored.
// `Deserialize` only — never sent on the wire. `Debug` for logs, `Clone` for cheap copy.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct Statistics {
    // `u64` not `usize`: wire counts must be platform-independent.
    pub(crate) total_documents: u64,

    // Missing key → 0. Forward-compatible if TipTap drops this field.
    #[serde(default)]
    pub(crate) current_loaded_documents_count: u64,

    // Capture for diagnostic logs; useful when TipTap's schema drifts.
    #[serde(default)]
    pub(crate) version: String,
}

/// A single TipTap document returned by `GET /api/documents`.
///
/// IMPORTANT: response shape is loosely documented. `name` IS guaranteed (it's
/// the identifier, equal to coaching_session UUID via
/// `coaching_sessions.collab_document_name`). Other fields are best-effort —
/// verify against a live response on first deploy.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct Document {
    // Join column for Sessions 5/6. Keep `String` (not `Uuid`) — TipTap treats
    // it as opaque; we should too until we explicitly parse for DB queries.
    pub(crate) name: String,

    // Approximate byte size. Field name is a best-guess — confirm against
    // live response. Default 0 → undercount on missing key beats a panic.
    #[serde(default)]
    pub(crate) size: u64,

    // Default `false` matches TipTap: only explicit archives are flagged.
    #[serde(default)]
    pub(crate) archived: bool,
}

// Paginated listing wrapper. TipTap returns a JSON array at the top level; if
// first probe reveals a wrapper (e.g. `{ "data": [...] }`), upgrade to a struct.
pub(crate) type DocumentsPage = Vec<Document>;

/// Build a reqwest::Client with TipTap auth headers and bounded timeouts.
/// Sibling pattern: `tiptap.rs::Client()` / `mailersend.rs::build_client()`,
/// plus explicit timeouts
//
// Free function (not method): lets tests exercise it without constructing
// a `Client` first.
async fn build_client(config: &Config) -> Result<reqwest::Client, Error> {
    let headers = build_auth_headers(config).await?;

    // `From<reqwest::Error> for Error` (domain/src/error.rs:107-126) maps
    // builder errors -> Internal::Other, network errors -> External::Network.
    // So `.build()?` Just Works.
    Ok(reqwest::Client::builder()
        // Pure-Rust TLS: no OpenSSL system dep in the Docker image.
        .use_rustls_tls()
        // Attaches headers to every request; cloned per-request, concurrency-safe.
        .default_headers(headers)
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()?)
}

/// Build the `Authorization` header for TipTap REST
///
/// IMPORTANT: TipTap auth is the Raw secret value - NOT `Bearer <secret>`.
/// Matches `tiptap.rs::build_auth_headers()`. Do NOT copy mailersend's
/// Bearer pattern blindly
async fn build_auth_headers(config: &Config) -> Result<reqwest::header::HeaderMap, Error> {
    // Same missing-config shape as Client::new, different field.
    let auth_key = config.tiptap_auth_key().ok_or_else(|| {
        warn!("TipTap auth key missing from config (metrics gateway init)");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    // `HeaderMap` is reqwest's case-insensitive header store.
    let mut headers = reqwest::header::HeaderMap::new();

    // `from_str` validates the bytes (no control chars). Defensive guard
    // against operators pasting a key with stray newlines. `mut` because
    // `set_sensitive` mutates next
    let mut auth_value = reqwest::header::HeaderValue::from_str(&auth_key).map_err(|err| {
        warn!("Failed to build TipTap auth header value: {err:?}");
        Error {
            // `Box::new` erases the concrete type into a trait object; preserves
            // the cause for log dumps via Error::source
            source: Some(Box::new(err)),
            // `Other` (not `Config`): code/data/ shape problem, distinct triage signal.
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Failed to create TipTap auth header value".to_string(),
            )),
        }
    })?;

    // Redact this value from `Debug` output - protects against accidental leaks
    // via dbg!, log lines, or panic backtraces, Free habit
    auth_value.set_sensitive(true);

    // Typed constant (not the string "Authorization"): typos become compile
    // errors, not 401s in production.
    headers.insert(reqwest::header::AUTHORIZATION, auth_value);

    // No Content-Type: GETs only; reqwest's default Accept is fine.
    Ok(headers)
}

// -----------------------------------------------
// Tests
// -----------------------------------------------

// Convention here: inline `#[cfg(test)] mod tests` at the bottom of each source
// file. Async tests use `#[tokio::test]`. HTTP is mocked via `mockito` (the
// only mock-server crate in this repo's dev-deps).
#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    /// Test-only Config factory. Mirrors `coaching_session.rs::test_config`.
    /// `Config::from_args` parses clap args; passing `--key=value` populates
    /// fields without touching env vars.
    fn test_config(tiptap_url: &str) -> Config {
        Config::from_args([
            "test",
            "--tiptap-auth-key=test-auth-key",
            &format!("--tiptap-url={tiptap_url}"),
        ])
    }

    /// Happy path: TipTap returns 200 + well-formed JSON, we get back a
    /// populated `Statistics`. Exercises auth-header plumbing, URL
    /// construction, status check, and rename_all deserialization in one shot
    #[tokio::test]
    async fn fetch_statistics_happy_path() -> Result<(), Error> {
        // `Server::new_async` binds a random port. The `_async` variant
        // is required inside a tokio runtime.
        let mut server = Server::new_async().await;

        // Register: GET /api/statistics -> 200 + JSON body. The `_mock`
        // binding owns the registration - dropping it before assertions
        // would un-register the route. Use `_mock`, Not `_`.
        let _mock = server
            .mock("GET", "/api/statistics")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "totalDocuments": 42,
                    "currentLoadedDocumentsCount": 7,
                    "version": "1.2.3"
                    }"#,
            )
            .create_async()
            .await;

        // Build a Client pointed at the mock server.
        let config = test_config(&server.url());
        let client = Client::new(&config).await?;

        let stats = client.fetch_statistics().await?;

        // Assertions cover shape And the rename_all camelCase mapping
        assert_eq!(stats.total_documents, 42);
        assert_eq!(stats.current_loaded_documents_count, 7);
        assert_eq!(stats.version, "1.2.3");

        Ok(())
    }

    /// Build a JSON array of N synthetic documents starting at `start`.
    /// `serde_json::json!` is the idiomatic way to construct JSON literals
    /// in tests - type-checked at compile time, no escaping headaches.
    fn make_documents_page(start: usize, count: usize) -> String {
        let docs: Vec<_> = (0..count)
            .map(|i| {
                serde_json::json!({
                    "name": format!("doc-{}", start + i),
                    "size": 1024,
                    "archived": false,
                })
            })
            .collect();
        serde_json::Value::Array(docs).to_string()
    }

    /// Two-page pagination: first page is full (100 docs -> keep going);
    /// second page is short (10 docs -> terminate). Verifies we collect
    /// all 110 across the call boundary, and that `skip` advances.
    #[tokio::test]
    async fn list_all_documents_paginates_until_short_page() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        // First page: skip=0&take=100 -> 100 docs
        let _page1 = server
            .mock("GET", "/api/documents")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("skip".into(), "0".into()),
                mockito::Matcher::UrlEncoded("take".into(), "100".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(make_documents_page(0, 100))
            .create_async()
            .await;

        // Second page: skip=100&take=100 -> 10 docs (short -> terminate).
        let _page2 = server
            .mock("GET", "/api/documents")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("skip".into(), "100".into()),
                mockito::Matcher::UrlEncoded("take".into(), "100".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(make_documents_page(100, 10))
            .create_async()
            .await;

        let config = test_config(&server.url());
        let client = Client::new(&config).await?;

        let docs = client.list_all_documents().await?;

        // Total count + first/last name verify ordering and completeness
        assert_eq!(docs.len(), 110);
        assert_eq!(docs[0].name, "doc-0");
        assert_eq!(docs[109].name, "doc-109");

        Ok(())
    }
}
