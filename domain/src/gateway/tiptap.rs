use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use log::*;
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
    let auth_key = config.tip_tap_auth_key().ok_or_else(|| {
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
