//! Svix webhook signature verification extractor.

use crate::AppState;
use axum::{
    async_trait,
    body::Bytes,
    extract::{FromRequest, Request},
    http::StatusCode,
};
use log::warn;
use meeting_auth::webhook::{SvixValidator, Validator};
use std::collections::HashMap;

/// Extractor that verifies a Svix-signed webhook request.
///
/// Rejects with 401 if the secret is missing/misconfigured or the signature is invalid.
/// On success, yields the raw request body bytes for downstream JSON parsing.
pub(crate) struct SvixSignature(pub Bytes);

#[async_trait]
impl FromRequest<AppState> for SvixSignature {
    type Rejection = (StatusCode, String);

    async fn from_request(req: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let secret = state.config.recall_ai_webhook_secret().ok_or_else(|| {
            warn!("RECALL_AI_WEBHOOK_SECRET not configured — rejecting webhook");
            (
                StatusCode::UNAUTHORIZED,
                "Webhook secret not configured".to_string(),
            )
        })?;

        let validator = SvixValidator::new("recall_ai".to_string(), &secret).map_err(|e| {
            warn!("Failed to build SvixValidator: {:?}", e);
            (
                StatusCode::UNAUTHORIZED,
                "Invalid webhook secret".to_string(),
            )
        })?;

        let header_map: HashMap<String, String> = req
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|val| (k.as_str().to_lowercase(), val.to_string()))
            })
            .collect();

        let body = Bytes::from_request(req, state).await.map_err(|e| {
            warn!("Failed to read webhook body: {:?}", e);
            (StatusCode::BAD_REQUEST, "Failed to read body".to_string())
        })?;

        match validator.validate(&header_map, &body) {
            Ok(true) => Ok(SvixSignature(body)),
            Ok(false) => {
                warn!(
                    "Invalid Svix signature: provider=recall_ai svix-id={}",
                    header_map
                        .get("svix-id")
                        .or_else(|| header_map.get("webhook-id"))
                        .map(|s| s.as_str())
                        .unwrap_or("")
                );
                Err((StatusCode::UNAUTHORIZED, "Invalid signature".to_string()))
            }
            Err(e) => {
                warn!(
                    "Svix validation error: provider=recall_ai svix-id={} error={:?}",
                    header_map
                        .get("svix-id")
                        .or_else(|| header_map.get("webhook-id"))
                        .map(|s| s.as_str())
                        .unwrap_or(""),
                    e
                );
                Err((
                    StatusCode::UNAUTHORIZED,
                    "Signature validation failed".to_string(),
                ))
            }
        }
    }
}
