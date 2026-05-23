//! Svix webhook signature verification extractor.

use crate::AppState;
use axum::{
    async_trait,
    body::Bytes,
    extract::{FromRequest, Request},
    http::StatusCode,
};
use log::warn;
use meeting_auth::webhook::svix::Validator as SvixValidator;
use meeting_auth::webhook::Validator;
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

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use axum::{body::Body, routing::post, Router};
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use hmac::{Hmac, Mac};
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use sha2::Sha256;
    use std::sync::Arc;
    use tower::ServiceExt;

    type HmacSha256 = Hmac<Sha256>;

    fn sign(key: &[u8], id: &str, timestamp: i64, body: &[u8]) -> String {
        let content = format!("{}.{}.", id, timestamp);
        let mut bytes = content.into_bytes();
        bytes.extend_from_slice(body);
        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(&bytes);
        format!("v1,{}", BASE64.encode(mac.finalize().into_bytes()))
    }

    fn build_test_app(secret: &str) -> Router {
        let config =
            Config::from_args(["refactor-platform-rs", "--recall-ai-webhook-secret", secret]);
        let db = Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection());
        let service_state = service::AppState::new(config, &db);
        let app_state = AppState::new(
            service_state,
            Arc::new(sse::Manager::new()),
            domain::events::EventPublisher::default(),
            None,
            None,
        );

        async fn handler(SvixSignature(_): SvixSignature) -> &'static str {
            "ok"
        }

        Router::new()
            .route("/webhook", post(handler))
            .with_state(app_state)
    }

    #[tokio::test]
    async fn webhook_alias_headers_are_accepted() {
        let raw_key = b"webhook_alias_test_key_32_bytes!";
        let secret = format!("whsec_{}", BASE64.encode(raw_key));
        let app = build_test_app(&secret);

        let body = b"{\"event\":\"recording.done\"}";
        let id = "wh_alias_extractor";
        let ts = chrono::Utc::now().timestamp();
        let sig = sign(raw_key, id, ts, body);

        let req = axum::http::Request::builder()
            .uri("/webhook")
            .method("POST")
            .header("webhook-id", id)
            .header("webhook-timestamp", ts.to_string())
            .header("webhook-signature", sig)
            .header("content-type", "application/json")
            .body(Body::from(body.to_vec()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn invalid_signature_returns_401() {
        let raw_key = b"webhook_alias_test_key_32_bytes!";
        let secret = format!("whsec_{}", BASE64.encode(raw_key));
        let app = build_test_app(&secret);

        let body = b"{\"event\":\"recording.done\"}";
        let ts = chrono::Utc::now().timestamp();

        let req = axum::http::Request::builder()
            .uri("/webhook")
            .method("POST")
            .header("svix-id", "msg_bad_sig")
            .header("svix-timestamp", ts.to_string())
            .header("svix-signature", "v1,invalidsignature==")
            .header("content-type", "application/json")
            .body(Body::from(body.to_vec()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_secret_config_returns_401() {
        let config = Config::default();
        let db = Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection());
        let service_state = service::AppState::new(config, &db);
        let app_state = AppState::new(
            service_state,
            Arc::new(sse::Manager::new()),
            domain::events::EventPublisher::default(),
            None,
            None,
        );

        async fn handler(SvixSignature(_): SvixSignature) -> &'static str {
            "ok"
        }

        let app = Router::new()
            .route("/webhook", post(handler))
            .with_state(app_state);

        let req = axum::http::Request::builder()
            .uri("/webhook")
            .method("POST")
            .header("svix-id", "msg_no_secret")
            .header("svix-timestamp", chrono::Utc::now().timestamp().to_string())
            .header("svix-signature", "v1,anysig")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
