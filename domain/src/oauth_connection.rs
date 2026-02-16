use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::gateway::oauth::{self, Provider};
use crate::oauth_connections::Model as OauthConnectionModel;
use crate::provider::Provider as OauthProvider;
use crate::Id;
use entity_api::oauth_connection;
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;

pub use entity_api::oauth_connection::{delete_by_user_and_provider, find_by_user_and_provider};

/// Build the Google OAuth authorization URL for a user.
pub fn google_authorize_url(config: &Config, user_id: Id) -> Result<String, Error> {
    let client_id = config.google_client_id().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    let redirect_uri = config.google_redirect_uri().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    let provider = oauth::google::new_provider(client_id, String::new(), redirect_uri);
    let state = user_id.to_string();
    let auth_request = provider.authorization_url(&state, None);

    info!("Redirecting user {} to Google OAuth", user_id);
    Ok(auth_request.url)
}

/// Exchange an authorization code for tokens and store them in oauth_connections.
///
/// Returns the success redirect URL for the frontend.
pub async fn exchange_and_store_tokens(
    db: &DatabaseConnection,
    config: &Config,
    user_id: Id,
    authorization_code: &str,
) -> Result<String, Error> {
    info!("Processing Google OAuth callback for user {}", user_id);

    let provider = create_google_provider(config)?;

    // Exchange authorization code for tokens
    let tokens = provider
        .exchange_code(authorization_code, None)
        .await
        .map_err(|e| {
            warn!(
                "Failed to exchange OAuth code for user {}: {:?}",
                user_id, e
            );
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Failed to complete Google authorization".to_string(),
                )),
            }
        })?
        .into_plain();

    // Get user info from Google
    let user_info = provider
        .get_user_info(&tokens.access_token)
        .await
        .map_err(|e| {
            warn!(
                "Failed to get Google user info for user {}: {:?}",
                user_id, e
            );
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Failed to get Google user info".to_string(),
                )),
            }
        })?;

    // Upsert oauth_connection
    let existing =
        oauth_connection::find_by_user_and_provider(db, user_id, OauthProvider::Google).await?;

    match existing {
        Some(conn) => {
            oauth_connection::update_tokens(
                db,
                conn.id,
                tokens.access_token,
                tokens.refresh_token,
                tokens.expires_at,
            )
            .await?;
        }
        None => {
            let now = chrono::Utc::now();
            let model = OauthConnectionModel {
                id: Id::new_v4(),
                user_id,
                provider: OauthProvider::Google,
                external_account_id: None,
                external_email: Some(user_info.email),
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
                token_expires_at: tokens.expires_at.map(|dt| dt.into()),
                token_type: "Bearer".to_string(),
                scopes: "openid email https://www.googleapis.com/auth/meetings.space.created"
                    .to_string(),
                created_at: now.into(),
                updated_at: now.into(),
            };
            oauth_connection::create(db, model).await?;
        }
    }

    info!(
        "Successfully stored Google OAuth tokens for user {}",
        user_id
    );

    let base_url = config.google_oauth_success_redirect_uri();
    Ok(format!("{}?google=connected", base_url))
}

/// Get a valid (non-expired) access token for a user and provider.
///
/// Refreshes the token if expired, storing the new tokens.
pub async fn get_valid_access_token(
    db: &DatabaseConnection,
    config: &Config,
    user_id: Id,
    provider: OauthProvider,
) -> Result<String, Error> {
    let connection = oauth_connection::find_by_user_and_provider(db, user_id, provider).await?;

    let connection = connection.ok_or_else(|| {
        warn!(
            "User {} has no OAuth connection for the requested provider",
            user_id
        );
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Please connect your account first in Settings > Integrations".to_string(),
            )),
        }
    })?;

    // Check if token is expired
    let is_expired = connection
        .token_expires_at
        .as_ref()
        .is_some_and(|expiry| *expiry < chrono::Utc::now().fixed_offset());

    if !is_expired {
        return Ok(connection.access_token);
    }

    // Token is expired, try to refresh
    let refresh_token = connection.refresh_token.as_ref().ok_or_else(|| {
        warn!(
            "OAuth token expired and no refresh token available for user {}",
            user_id
        );
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Authorization expired. Please reconnect your account.".to_string(),
            )),
        }
    })?;

    let oauth_provider = create_google_provider(config)?;
    let refresh_result = oauth_provider
        .refresh_token(refresh_token)
        .await
        .map_err(|e| {
            warn!(
                "Failed to refresh OAuth token for user {}: {:?}",
                user_id, e
            );
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Failed to refresh authorization. Please reconnect your account.".to_string(),
                )),
            }
        })?;

    let tokens = refresh_result.tokens.into_plain();

    oauth_connection::update_tokens(
        db,
        connection.id,
        tokens.access_token.clone(),
        tokens.refresh_token,
        tokens.expires_at,
    )
    .await?;

    Ok(tokens.access_token)
}

/// Create a Google OAuth provider from config.
fn create_google_provider(config: &Config) -> Result<impl Provider, Error> {
    let client_id = config.google_client_id().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    let client_secret = config.google_client_secret().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    let redirect_uri = config.google_redirect_uri().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    Ok(oauth::google::new_provider(
        client_id,
        client_secret,
        redirect_uri,
    ))
}
