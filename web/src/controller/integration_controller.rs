//! Controller for user integration management.
//!
//! Handles API key storage and verification for external services
//! (Recall.ai, AssemblyAI). Google OAuth is handled separately.

use crate::controller::ApiResponse;
use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::params::integration::{
    IntegrationStatusResponse, UpdateIntegrationParams, VerifyApiKeyResponse,
};
use crate::{AppState, Error};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use domain::gateway::assembly_ai::AssemblyAiClient;
use domain::gateway::recall_ai::RecallAiClient;
use domain::user_integrations::Model as UserIntegrationModel;
use domain::{user_integration, Id};
use service::config::ApiVersion;

/// GET user integration status
///
/// Returns the integration configuration status for a user without exposing API keys.
#[utoipa::path(
    get,
    path = "/users/{user_id}/integrations",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID"),
    ),
    responses(
        (status = 200, description = "Integration status retrieved", body = IntegrationStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "User not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    let integration: UserIntegrationModel =
        user_integration::get_or_create(app_state.db_conn_ref(), user_id).await?;
    let response: IntegrationStatusResponse = integration.into();
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), response)))
}

/// PUT update user integrations
///
/// Updates API keys for external services. Keys are encrypted at rest.
#[utoipa::path(
    put,
    path = "/users/{user_id}/integrations",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID"),
    ),
    request_body = UpdateIntegrationParams,
    responses(
        (status = 200, description = "Integration updated", body = IntegrationStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "User not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
    Json(params): Json<UpdateIntegrationParams>,
) -> Result<impl IntoResponse, Error> {
    let mut integration: UserIntegrationModel =
        user_integration::get_or_create(app_state.db_conn_ref(), user_id).await?;

    // Update only provided fields
    if let Some(key) = params.recall_ai_api_key {
        integration.recall_ai_api_key = Some(key);
        // Reset verification status when key changes
        integration.recall_ai_verified_at = None;
    }
    if let Some(region) = params.recall_ai_region {
        integration.recall_ai_region = Some(region);
    }
    if let Some(key) = params.assembly_ai_api_key {
        integration.assembly_ai_api_key = Some(key);
        // Reset verification status when key changes
        integration.assembly_ai_verified_at = None;
    }

    let updated: UserIntegrationModel =
        user_integration::update(app_state.db_conn_ref(), integration.id, integration).await?;
    let response: IntegrationStatusResponse = updated.into();
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), response)))
}

/// POST verify Recall.ai API key
///
/// Tests if the configured Recall.ai API key is valid.
#[utoipa::path(
    post,
    path = "/users/{user_id}/integrations/verify/recall-ai",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID"),
    ),
    responses(
        (status = 200, description = "Verification result", body = VerifyApiKeyResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "User not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn verify_recall_ai(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    let mut integration: UserIntegrationModel =
        user_integration::get_or_create(app_state.db_conn_ref(), user_id).await?;

    let api_key = match &integration.recall_ai_api_key {
        Some(key) => key,
        None => {
            return Ok(Json(ApiResponse::new(
                StatusCode::OK.into(),
                VerifyApiKeyResponse {
                    valid: false,
                    message: Some("No Recall.ai API key configured".to_string()),
                },
            )));
        }
    };

    let region = integration
        .recall_ai_region
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();

    let config = &app_state.config;
    let client = RecallAiClient::new(api_key, region, config.recall_ai_base_domain())?;
    let valid = client.verify_api_key().await?;

    if valid {
        // Update verification timestamp
        integration.recall_ai_verified_at = Some(chrono::Utc::now().into());
        let _: UserIntegrationModel =
            user_integration::update(app_state.db_conn_ref(), integration.id, integration).await?;
    }

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        VerifyApiKeyResponse {
            valid,
            message: if valid {
                Some("API key verified successfully".to_string())
            } else {
                Some("API key is invalid".to_string())
            },
        },
    )))
}

/// POST verify AssemblyAI API key
///
/// Tests if the configured AssemblyAI API key is valid.
#[utoipa::path(
    post,
    path = "/users/{user_id}/integrations/verify/assembly-ai",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID"),
    ),
    responses(
        (status = 200, description = "Verification result", body = VerifyApiKeyResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "User not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn verify_assembly_ai(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    let mut integration: UserIntegrationModel =
        user_integration::get_or_create(app_state.db_conn_ref(), user_id).await?;

    let api_key = match &integration.assembly_ai_api_key {
        Some(key) => key,
        None => {
            return Ok(Json(ApiResponse::new(
                StatusCode::OK.into(),
                VerifyApiKeyResponse {
                    valid: false,
                    message: Some("No AssemblyAI API key configured".to_string()),
                },
            )));
        }
    };

    let config = &app_state.config;
    let client = AssemblyAiClient::new(api_key, config.assembly_ai_base_url())?;
    let valid = client.verify_api_key().await?;

    if valid {
        // Update verification timestamp
        integration.assembly_ai_verified_at = Some(chrono::Utc::now().into());
        let _: UserIntegrationModel =
            user_integration::update(app_state.db_conn_ref(), integration.id, integration).await?;
    }

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        VerifyApiKeyResponse {
            valid,
            message: if valid {
                Some("API key verified successfully".to_string())
            } else {
                Some("API key is invalid".to_string())
            },
        },
    )))
}

/// DELETE disconnect Google account
///
/// Removes Google OAuth tokens from user's integration.
#[utoipa::path(
    delete,
    path = "/users/{user_id}/integrations/google",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID"),
    ),
    responses(
        (status = 200, description = "Google account disconnected", body = IntegrationStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "User not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn disconnect_google(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    let mut integration: UserIntegrationModel =
        user_integration::get_or_create(app_state.db_conn_ref(), user_id).await?;

    // Clear Google OAuth fields
    integration.google_access_token = None;
    integration.google_refresh_token = None;
    integration.google_token_expiry = None;
    integration.google_email = None;

    let updated: UserIntegrationModel =
        user_integration::update(app_state.db_conn_ref(), integration.id, integration).await?;
    let response: IntegrationStatusResponse = updated.into();
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), response)))
}
