//! Controller for OAuth authentication flows and connection management.
//!
//! Handles Google OAuth for Google Meet integration.
//!
//! Note: The authorize/callback endpoints don't use CompareApiVersion because they work via
//! browser redirects which cannot set custom headers.

use crate::controller::ApiResponse;
use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::{AppState, Error};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect};
use axum::Json;

use domain::{oauth_connection, oauth_connections, provider::Provider, Id};
use sea_orm::prelude::DateTimeWithTimeZone;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::error::WebErrorKind;

/// Query parameters for starting OAuth
#[derive(Debug, Deserialize)]
pub struct OAuthStart {
    pub user_id: Id,
}

/// Lean projection of an oauth_connections row — tokens are never exposed.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectionResponse {
    pub provider: Provider,
    pub email: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub connected_at: DateTimeWithTimeZone,
}

impl From<oauth_connections::Model> for ConnectionResponse {
    fn from(m: oauth_connections::Model) -> Self {
        Self {
            provider: m.provider,
            email: m.external_email,
            connected_at: m.created_at,
        }
    }
}

/// GET /oauth/:provider/authorize
///
/// Initiates a Provider's OAuth flow by redirecting to their authorization endpoint.
/// Note: This endpoint doesn't require x-version header as it's called via browser redirect.
#[utoipa::path(
    get,
    path = "/oauth/{provider}/authorize",
    params(
        ("user_id" = Id, Query, description = "User ID to associate with Google account"),
    ),
    responses(
        (status = 302, description = "Redirect to Google OAuth"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Server error (OAuth not configured)"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn authorize(
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Query(params): Query<OAuthStart>,
    Path(provider): Path<Provider>,
) -> Result<impl IntoResponse, Error> {
    if user.id != params.user_id {
        return Err(Error::Web(WebErrorKind::Auth));
    }

    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_string(), params.user_id.to_string());
    let state_token = app_state.oauth_state_manager.generate(None, metadata);

    let url = oauth_connection::authorize_url(&app_state.config, &state_token, provider)?;

    Ok(Redirect::temporary(&url))
}

/// GET /oauth/connections
///
/// Returns all OAuth connections for the authenticated user.
#[utoipa::path(
    get,
    path = "/oauth/connections",
    responses(
        (status = 200, description = "List of OAuth connections", body = Vec<ConnectionResponse>),
        (status = 401, description = "Unauthorized"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let connections = oauth_connection::find_all_by_user(app_state.db_conn_ref(), user.id)
        .await?
        .into_iter()
        .map(ConnectionResponse::from)
        .collect::<Vec<_>>();

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), connections)))
}

/// GET /oauth/connections/:provider
///
/// Returns the OAuth connection for the authenticated user and given provider, or 404 if not connected.
#[utoipa::path(
    get,
    path = "/oauth/connections/{provider}",
    params(
        ("provider" = Provider, Path, description = "OAuth provider (e.g. google)"),
    ),
    responses(
        (status = 200, description = "OAuth connection found", body = ConnectionResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not connected to this provider"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(provider): Path<Provider>,
) -> Result<impl IntoResponse, Error> {
    let connection =
        oauth_connection::get_by_user_and_provider(app_state.db_conn_ref(), user.id, provider)
            .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        ConnectionResponse::from(connection),
    )))
}

/// DELETE /oauth/connections/:provider
///
/// Disconnects (deletes) the OAuth connection for the authenticated user and given provider.
#[utoipa::path(
    delete,
    path = "/oauth/connections/{provider}",
    params(
        ("provider" = Provider, Path, description = "OAuth provider (e.g. google)"),
    ),
    responses(
        (status = 204, description = "Disconnected successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not connected to this provider"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(provider): Path<Provider>,
) -> Result<impl IntoResponse, Error> {
    oauth_connection::delete_by_user_and_provider(app_state.db_conn_ref(), user.id, provider)
        .await?;

    Ok(Json(ApiResponse::<()>::no_content(
        StatusCode::NO_CONTENT.into(),
    )))
}
