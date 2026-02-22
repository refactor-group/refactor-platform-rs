use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::gateway::oauth::{self, Provider};
use crate::oauth_connections::Model as OauthConnectionModel;
use crate::oauth_token_storage::DbOAuthTokenStorage;
use crate::provider::Provider as OauthProvider;
use crate::Id;
use entity_api::oauth_connection;
use log::*;
use meeting_auth::oauth::token::{encryption, Manager};
use sea_orm::DatabaseConnection;
use secrecy::ExposeSecret;
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

    let encryption_key = config.encryption_key().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    let provider = create_google_provider(config)?;

    let tokens = provider
        .exchange_code(authorization_code, None)
        .await
        .inspect_err(|e| {
            warn!(
                "Failed to exchange OAuth code for user {}: {:?}",
                user_id, e
            )
        })?
        .into_plain();

    let user_info = provider
        .get_user_info(&tokens.access_token)
        .await
        .inspect_err(|e| {
            warn!(
                "Failed to get Google user info for user {}: {:?}",
                user_id, e
            )
        })?;

    let encrypted_access =
        encryption::encrypt(&tokens.access_token, &encryption_key).map_err(|e| Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Failed to encrypt access token".to_string(),
            )),
        })?;
    let encrypted_refresh = tokens
        .refresh_token
        .as_deref()
        .map(|rt| encryption::encrypt(rt, &encryption_key))
        .transpose()
        .map_err(|e: meeting_auth::Error| Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Failed to encrypt refresh token".to_string(),
            )),
        })?;

    let existing =
        oauth_connection::find_by_user_and_provider(db, user_id, OauthProvider::Google).await?;

    match existing {
        Some(conn) => {
            oauth_connection::update_tokens(
                db,
                conn.id,
                encrypted_access,
                encrypted_refresh,
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
                access_token: encrypted_access,
                refresh_token: encrypted_refresh,
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
/// Uses `Manager` for per-user refresh locking and automatic token refresh.
pub async fn get_valid_access_token(
    db: &DatabaseConnection,
    config: &Config,
    user_id: Id,
    _provider: OauthProvider,
) -> Result<String, Error> {
    let encryption_key = config.encryption_key().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    let oauth_provider = create_google_provider(config)?;
    let storage = DbOAuthTokenStorage::new(db, encryption_key);
    let manager = Manager::new(storage);

    let access_token = manager
        .get_valid_token(&oauth_provider, &user_id.to_string())
        .await
        .inspect_err(|e| warn!("Failed to get valid token for user {}: {:?}", user_id, e))?;

    Ok(access_token.expose_secret().to_string())
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
