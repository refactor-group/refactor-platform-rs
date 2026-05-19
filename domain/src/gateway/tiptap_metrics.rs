//! TipTap Cloud metrics gateway.
//!
//! Read-only methods over TipTap's REST surface: per-doc listings and
//! ingredients for abandoned-doc detection.

use std::time::Duration;

use log::*;
use serde::Deserialize;

use service::config::Config;

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};

// Admin endpoints need bounded budgets so a dead upstream can't hold an
// Axum worker. `tiptap.rs` intentionally sets none; this gateway differs.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

const PAGE_SIZE: u32 = 100;

// Safety cap: 1M docs at PAGE_SIZE=100. Bail with partial results rather
// than OOM if TipTap ever returns full pages forever.
const MAX_PAGES: u32 = 10_000;

pub(crate) struct Client {
    client: reqwest::Client,
    base_url: String,
}

impl Client {
    pub(crate) async fn new(config: &Config) -> Result<Self, Error> {
        let client = build_client(config).await?;

        let base_url = config.tiptap_url().ok_or_else(|| {
            warn!("TipTap URL missing from config (metrics gateway init)");
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
            }
        })?;

        Ok(Self { client, base_url })
    }

    async fn fetch_documents_page(&self, skip: u32, take: u32) -> Result<DocumentsPage, Error> {
        let url = format!("{}/api/documents", self.base_url);

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
            warn!("TipTap /api/documents returned {status} at skip={skip}: {body}");
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

    /// Fetch every TipTap document via offset pagination.
    ///
    /// Offset pagination is racy under concurrent writes — acceptable for
    /// admin observability where eventual consistency is fine.
    pub(crate) async fn list_all_documents(&self) -> Result<Vec<Document>, Error> {
        let mut all: Vec<Document> = Vec::new();
        let mut skip: u32 = 0;

        for _ in 0..MAX_PAGES {
            let page = self.fetch_documents_page(skip, PAGE_SIZE).await?;
            let page_len = page.len() as u32;

            all.extend(page);

            // TipTap has no `total` field; a short page is the only end-of-data signal.
            if page_len < PAGE_SIZE {
                return Ok(all);
            }

            skip = skip.saturating_add(PAGE_SIZE);
        }

        warn!(
            "TipTap document pagination hit MAX_PAGES={MAX_PAGES} cap; \
            returning {} partial results",
            all.len()
        );
        Ok(all)
    }
}

/// A single TipTap document returned by `GET /api/documents`.
///
/// `name` is the identifier; equals `coaching_sessions.collab_document_name`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct Document {
    pub(crate) name: String,

    #[serde(default)]
    pub(crate) size: u64,

    #[serde(default)]
    pub(crate) archived: bool,
}

pub(crate) type DocumentsPage = Vec<Document>;

async fn build_client(config: &Config) -> Result<reqwest::Client, Error> {
    let headers = build_auth_headers(config).await?;

    Ok(reqwest::Client::builder()
        .use_rustls_tls()
        .default_headers(headers)
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()?)
}

// TipTap auth is the raw secret value, NOT `Bearer <secret>`. Matches
// `tiptap.rs::build_auth_headers`; do not copy mailersend's Bearer pattern.
async fn build_auth_headers(config: &Config) -> Result<reqwest::header::HeaderMap, Error> {
    let auth_key = config.tiptap_auth_key().ok_or_else(|| {
        warn!("TipTap auth key missing from config (metrics gateway init)");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    let mut headers = reqwest::header::HeaderMap::new();

    let mut auth_value = reqwest::header::HeaderValue::from_str(&auth_key).map_err(|err| {
        warn!("Failed to build TipTap auth header value: {err:?}");
        Error {
            source: Some(Box::new(err)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Failed to create TipTap auth header value".to_string(),
            )),
        }
    })?;

    auth_value.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, auth_value);

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    fn test_config(tiptap_url: &str) -> Config {
        Config::from_args([
            "test",
            "--tiptap-auth-key=test-auth-key",
            &format!("--tiptap-url={tiptap_url}"),
        ])
    }

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

    #[tokio::test]
    async fn list_all_documents_paginates_until_short_page() -> Result<(), Error> {
        let mut server = Server::new_async().await;

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

        assert_eq!(docs.len(), 110);
        assert_eq!(docs[0].name, "doc-0");
        assert_eq!(docs[109].name, "doc-109");

        Ok(())
    }

    #[tokio::test]
    async fn list_all_documents_maps_5xx_to_external_network() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/documents")
            .match_query(mockito::Matcher::Any)
            .with_status(500)
            .with_body("upstream_broke")
            .create_async()
            .await;

        let config = test_config(&server.url());
        let client = Client::new(&config).await.expect("client builds");
        let err = client.list_all_documents().await.expect_err("expected Err");

        assert!(
            matches!(
                err.error_kind,
                DomainErrorKind::External(ExternalErrorKind::Network),
            ),
            "expected External::Network, got {:?}",
            err.error_kind,
        );
    }

    #[tokio::test]
    async fn list_all_documents_maps_bad_json_to_external_network() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/documents")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{ "nope": "this is not a documents page"}"#)
            .create_async()
            .await;

        let config = test_config(&server.url());
        let client = Client::new(&config).await.expect("client builds");
        let err = client.list_all_documents().await.expect_err("expected Err");

        assert!(matches!(
            err.error_kind,
            DomainErrorKind::External(ExternalErrorKind::Network),
        ));
    }
}
