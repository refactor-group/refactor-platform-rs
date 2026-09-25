//! Authentication strategies for TipTap's REST surface.
//!
//! A single `Authenticator` interface both gateways depend on. Header assembly
//! is composed with combinators rather than imperative mutation.

use log::warn;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

use crate::error::{DomainErrorKind, Error, InternalErrorKind};

/// Supplies the default headers that authenticate a TipTap HTTP client.
pub(crate) trait Authenticator {
    fn headers(&self) -> Result<HeaderMap, Error>;
}

/// The combinator every scheme funnels through: wrap a string as a sensitive
/// `Authorization` header map.
fn authorization(value: String) -> Result<HeaderMap, Error> {
    HeaderValue::from_str(&value)
        .map(|mut header| {
            header.set_sensitive(true);
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, header);
            headers
        })
        .map_err(|err| {
            warn!("Failed to build TipTap Authorization header: {err:?}");
            Error {
                source: Some(Box::new(err)),
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "invalid TipTap Authorization header value".to_string(),
                )),
            }
        })
}

/// Raw shared-secret auth: `Authorization: <secret>`. The scheme TipTap's
/// document server accepts for per-document operations (create/delete/export).
pub(crate) struct SecretAuth {
    secret: String,
}

impl SecretAuth {
    pub(crate) fn new(secret: String) -> Self {
        Self { secret }
    }
}

impl Authenticator for SecretAuth {
    fn headers(&self) -> Result<HeaderMap, Error> {
        authorization(self.secret.clone())
    }
}
