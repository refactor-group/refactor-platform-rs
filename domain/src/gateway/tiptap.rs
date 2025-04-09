use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use log::*;
use serde_json::json;
use service::config::Config;

/// HTTP client for making requests to Tiptap. This client is configured with the necessary
/// authentication headers to make authenticated requests to Tiptap.
pub(crate) async fn client(config: &Config) -> Result<reqwest::Client, Error> {
    let headers = build_auth_headers(config).await?;

    Ok(reqwest::Client::builder()
        .use_rustls_tls()
        .default_headers(headers)
        .build()?)
}

async fn build_auth_headers(config: &Config) -> Result<reqwest::header::HeaderMap, Error> {
    let auth_key = config.tiptap_auth_key().ok_or_else(|| {
        warn!("Failed to get auth key from config");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
        }
    })?;
    let mut headers = reqwest::header::HeaderMap::new();
    let mut auth_value = reqwest::header::HeaderValue::from_str(&auth_key).map_err(|err| {
        warn!("Failed to create auth header value: {:?}", err);
        Error {
            source: Some(Box::new(err)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
        }
    })?;
    auth_value.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, auth_value);
    Ok(headers)
}

pub struct TiptapDocument {
    client: reqwest::Client,
    base_url: String,
}

impl TiptapDocument {
    pub async fn new(config: &Config) -> Result<Self, Error> {
        let client = client(config).await?;
        let base_url = config.tiptap_url().ok_or_else(|| {
            warn!("Failed to get Tiptap URL from config");
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
            }
        })?;
        Ok(Self { client, base_url })
    }

    pub async fn create(&self, document_name: &str) -> Result<(), Error> {
        let url = self.format_url(document_name);
        let response = self
            .client
            .post(url)
            .json(&json!({"type": "doc", "content": []}))
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to send request: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() || response.status().as_u16() == 409 {
            Ok(())
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Failed to create Tiptap document: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            })
        }
    }

    pub async fn delete(&self, document_name: &str) -> Result<(), Error> {
        let url = self.format_url(document_name);
        let response = self.client.delete(url).send().await.map_err(|e| {
            warn!("Failed to send request: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        let status = response.status();
        if status.is_success() || status.as_u16() == 404 {
            Ok(())
        } else {
            warn!(
                "Failed to delete Tiptap document: {}, with status: {}",
                document_name, status
            );
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            })
        }
    }

    fn format_url(&self, document_name: &str) -> String {
        format!(
            "{}/api/documents/{}?format=json",
            self.base_url, document_name
        )
    }
}
