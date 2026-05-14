//! Webhook handler for Recall.ai events.
//!
//! Recall.ai retries delivery on any non-2xx response, so permanent errors
//! (malformed payload, unrecognised event type) are acknowledged with 200 to
//! suppress retries. Transient failures return 500 so Recall.ai will retry.
//!
//! - 200 OK: event processed, idempotent skip, or unrecoverable parse/validation error
//! - 401 Unauthorized: Svix signature invalid (handled by extractor; Svix will not retry)
//! - 500 Internal Server Error: transient DB or infrastructure failure; Recall.ai will retry

use crate::extractors::svix_signature::SvixSignature;
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use log::*;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct WebhookEvent {
    event: String,
    data: Value,
}

/// POST /webhooks/recall_ai — receives all Recall.ai webhook events
pub async fn recall_ai(
    State(app_state): State<AppState>,
    SvixSignature(body): SvixSignature,
) -> impl IntoResponse {
    let raw: WebhookEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            warn!("Recall.ai webhook: malformed body (permanent, not retrying): {:?}", e);
            return StatusCode::OK.into_response();
        }
    };

    let event = match domain::webhook::Event::parse(&raw.event, raw.data) {
        Ok(e) => e,
        Err(e) => {
            warn!("Recall.ai webhook: unrecognised event type (permanent, not retrying): {:?}", e);
            return StatusCode::OK.into_response();
        }
    };

    match domain::webhook::dispatch(
        &app_state.database_connection,
        &app_state.config,
        &app_state.event_publisher,
        event,
    )
    .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => {
            error!("Recall.ai webhook error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
