//! Webhook handler for Recall.ai events.
//!
//! Response code semantics for Svix retry logic:
//! - 200 OK: event received and processed (or idempotent skip)
//! - 400 Bad Request: payload is malformed; Svix will not retry
//! - 401 Unauthorized: signature invalid; Svix will not retry
//! - 500 Internal Server Error: DB or infrastructure failure; Svix will retry

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
            warn!("Failed to deserialize Recall.ai webhook body: {:?}", e);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let event = match domain::webhook::Event::parse(&raw.event, raw.data) {
        Ok(e) => e,
        Err(e) => {
            warn!("Recall.ai webhook: invalid payload: {:?}", e);
            return StatusCode::BAD_REQUEST.into_response();
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
