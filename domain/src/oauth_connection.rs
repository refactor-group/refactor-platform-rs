use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use crate::gateway::oauth::{self, Provider};
use crate::meeting_provider::Provider as MeetingProvider;
use crate::oauth_connections::Model as OauthConnectionModel;
use crate::oauth_token_storage::DbOAuthTokenStorage;
use crate::Id;
use entity_api::oauth_connection as ConnectionApi;
use log::*;
use meeting_auth::oauth::token::{encryption, Manager, Plain};
use meeting_auth::oauth::UserInfo;
use sea_orm::DatabaseConnection;
use secrecy::{ExposeSecret, SecretString};
use service::config::Config;

pub use entity_api::oauth_connection::{
    find_all_by_user, find_by_user, find_by_user_and_provider, get_by_user_and_provider,
};

/// Build the Provider's OAuth authorization URL with the given CSRF state token.
pub fn authorize_url(
    config: &Config,
    state: &str,
    provider: MeetingProvider,
) -> Result<String, Error> {
    let auth_request = create_provider(config, provider)?.authorization_url(state, None);

    Ok(auth_request.url)
}

/// Exchange an authorization code for a Provider's tokens and store them in oauth_connections.
///
/// Returns the success redirect URL for the frontend.
pub async fn exchange_and_store_tokens(
    db: &DatabaseConnection,
    config: &Config,
    user_id: Id,
    authorization_code: &str,
    provider: MeetingProvider,
) -> Result<String, Error> {
    info!(
        "Processing {} OAuth callback for user {}",
        provider, user_id
    );

    let encryption_key = SecretString::from(config.encryption_key().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?);

    let oauth_provider = create_provider(config, provider)?;

    let tokens_raw = oauth_provider
        .exchange_code(authorization_code, None)
        .await
        .inspect_err(|e| {
            warn!(
                "Failed to exchange OAuth code for user {}: {:?}",
                user_id, e
            )
        })?;
    let scopes = tokens_raw.scopes.join(" ");
    let tokens = tokens_raw.into_plain();

    let user_info = oauth_provider
        .get_user_info(&tokens.access_token)
        .await
        .inspect_err(|e| {
            warn!(
                "Failed to get {} user info for user {}: {:?}",
                provider, user_id, e
            )
        })?;

    let encrypted_access =
        encryption::encrypt(&tokens.access_token, encryption_key.expose_secret()).map_err(|e| {
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Failed to encrypt access token".to_string(),
                )),
            }
        })?;
    let encrypted_refresh = tokens
        .refresh_token
        .as_deref()
        .map(|rt| encryption::encrypt(rt, encryption_key.expose_secret()))
        .transpose()
        .map_err(|e: meeting_auth::Error| Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Failed to encrypt refresh token".to_string(),
            )),
        })?;

    let existing = ConnectionApi::find_by_user_and_provider(db, user_id, provider).await?;

    match existing {
        Some(conn) => {
            ConnectionApi::update_tokens(
                db,
                conn.id,
                encrypted_access,
                encrypted_refresh,
                tokens.expires_at,
            )
            .await?;
        }
        None => {
            let model = create_oauth_connection_model(
                user_id,
                provider,
                user_info,
                &tokens,
                scopes,
                encrypted_access,
                encrypted_refresh,
            );
            ConnectionApi::create(db, model).await?;
        }
    }

    info!(
        "Successfully stored {} OAuth tokens for user {}",
        provider, user_id
    );

    let base_url = config.oauth_success_redirect_uri();

    Ok(format!(
        "{}?{}=connected",
        base_url,
        provider.to_string().to_lowercase()
    ))
}

/// Get a valid (non-expired) access token for a user and provider.
///
/// Uses `Manager` for per-user refresh locking and automatic token refresh.
pub async fn get_valid_access_token(
    db: &DatabaseConnection,
    config: &Config,
    user_id: Id,
    provider: MeetingProvider,
) -> Result<String, Error> {
    let encryption_key = SecretString::from(config.encryption_key().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?);

    let storage = DbOAuthTokenStorage::new(db, encryption_key);
    let manager = Manager::new(storage);

    let result = match provider {
        MeetingProvider::Google => {
            let oauth_provider = create_google_provider(config)?;
            manager
                .get_valid_token(&oauth_provider, &user_id.to_string())
                .await
                .inspect_err(|e| {
                    warn!(
                        "Failed to get valid google token for user {}: {:?}",
                        user_id, e
                    )
                })
        }
        MeetingProvider::Zoom => {
            let oauth_provider = create_zoom_provider(config)?;
            manager
                .get_valid_token(&oauth_provider, &user_id.to_string())
                .await
                .inspect_err(|e| {
                    warn!(
                        "Failed to get valid zoom token for user {}: {:?}",
                        user_id, e
                    )
                })
        }
    };

    match result {
        Ok(token) => Ok(token.expose_secret().to_string()),
        Err(e)
            if matches!(
                e.error_kind,
                meeting_auth::error::ErrorKind::OAuth(
                    meeting_auth::error::OAuthErrorKind::TokenRevoked
                )
            ) =>
        {
            warn!(
                "Refresh token revoked for user {}, removing connection",
                user_id
            );
            // The provider already revoked this grant, so drop the row without a revoke round trip.
            if let Err(del_err) =
                ConnectionApi::delete_by_user_and_provider(db, user_id, provider).await
            {
                warn!(
                    "Failed to delete revoked OAuth connection for user {}: {:?}",
                    user_id, del_err
                );
            }
            Err(Error {
                error_kind: DomainErrorKind::External(ExternalErrorKind::OauthTokenRevoked(
                    provider.to_string().to_lowercase(),
                )),
                source: Some(Box::new(e)),
            })
        }
        Err(e) => Err(e.into()),
    }
}

/// Disconnect a user from a provider: revoke the stored grant, then delete the connection.
///
/// Revocation is best effort. It is attempted first so that a token is never orphaned at the
/// provider, but a failure is logged rather than propagated: leaving the row behind would strand
/// the user in a connected-but-broken state with no way to retry. In the reverse case, where the
/// grant is revoked but the delete fails, the next token use fails as `TokenRevoked` and
/// `get_valid_access_token` drops the row then.
///
/// # Errors
///
/// Returns `RecordNotFound` when the user has no connection for `provider`.
pub async fn delete_by_user_and_provider(
    db: &DatabaseConnection,
    config: &Config,
    user_id: Id,
    provider: MeetingProvider,
) -> Result<(), Error> {
    let connection = ConnectionApi::get_by_user_and_provider(db, user_id, provider).await?;

    revoke_stored_tokens(config, &connection, provider).await;

    // Delete by id rather than re-resolving the pair, so a concurrent disconnect that already
    // removed the row still succeeds instead of surfacing a 404 after a completed revoke.
    ConnectionApi::delete_by_id(db, connection.id).await?;

    info!("Disconnected {} for user {}", provider, user_id);

    Ok(())
}

/// Revoke a connection's stored grant with the provider, logging and returning on any failure.
///
/// Tries the refresh token first, since it outlives the access token, and falls back to the access
/// token. Google revokes the whole grant given either, while Zoom documents only the access token,
/// so trying both is what makes one code path correct for both providers.
async fn revoke_stored_tokens(
    config: &Config,
    connection: &OauthConnectionModel,
    provider: MeetingProvider,
) {
    let user_id = connection.user_id;

    let Some(encryption_key) = config.encryption_key() else {
        warn!("Cannot revoke {provider} token for user {user_id}: no encryption key configured");
        return;
    };

    let tokens = revocable_tokens(connection, &encryption_key);
    if tokens.is_empty() {
        warn!("Cannot revoke {provider} token for user {user_id}: no token could be decrypted");
        return;
    }

    let oauth_provider = match create_provider(config, provider) {
        Ok(oauth_provider) => oauth_provider,
        Err(e) => {
            warn!("Cannot revoke {provider} token for user {user_id}: {e:?}");
            return;
        }
    };

    for token in &tokens {
        match oauth_provider.revoke_token(token).await {
            Ok(()) => {
                info!("Revoked {provider} grant for user {user_id}");
                return;
            }
            Err(e) => warn!("Failed to revoke {provider} token for user {user_id}: {e:?}"),
        }
    }
}

/// The connection's decryptable tokens, refresh first, skipping any that fail to decrypt.
fn revocable_tokens(connection: &OauthConnectionModel, encryption_key: &str) -> Vec<String> {
    [
        connection.refresh_token.as_deref(),
        Some(connection.access_token.as_str()),
    ]
    .into_iter()
    .flatten()
    .filter_map(|stored| encryption::decrypt(stored, encryption_key).ok())
    .collect()
}

/// Create the OAuth provider implementation backing a meeting provider.
fn create_provider(config: &Config, provider: MeetingProvider) -> Result<Box<dyn Provider>, Error> {
    match provider {
        MeetingProvider::Google => Ok(Box::new(create_google_provider(config)?)),
        MeetingProvider::Zoom => Ok(Box::new(create_zoom_provider(config)?)),
    }
}

/// Create a Google OAuth provider from config.
fn create_google_provider(config: &Config) -> Result<impl Provider, Error> {
    let client_id = config.google_client_id().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    let client_secret = SecretString::from(config.google_client_secret().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?);

    let redirect_uri = config.google_redirect_uri().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    Ok(oauth::google::new_provider(
        client_id,
        client_secret,
        redirect_uri,
    )?)
}

/// Create a Zoom OAuth provider from config.
fn create_zoom_provider(config: &Config) -> Result<impl Provider, Error> {
    let client_id = config.zoom_client_id().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    let client_secret = SecretString::from(config.zoom_client_secret().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?);

    let redirect_uri = config.zoom_redirect_uri().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    })?;

    Ok(oauth::zoom::new_provider(
        client_id,
        client_secret,
        redirect_uri,
    )?)
}

fn create_oauth_connection_model(
    user_id: Id,
    provider: MeetingProvider,
    user_info: UserInfo,
    tokens: &Plain,
    scopes: String,
    encrypted_access: String,
    encrypted_refresh: Option<String>,
) -> OauthConnectionModel {
    let now = chrono::Utc::now();

    // Start with common fields
    let mut model = OauthConnectionModel {
        id: Id::new_v4(),
        user_id,
        provider,
        external_account_id: None,
        external_email: None,
        access_token: encrypted_access,
        refresh_token: encrypted_refresh,
        token_expires_at: tokens.expires_at.map(|dt| dt.into()),
        token_type: "Bearer".to_string(),
        scopes,
        created_at: now.into(),
        updated_at: now.into(),
    };

    match provider {
        MeetingProvider::Google => apply_google_fields(&mut model, user_info),
        MeetingProvider::Zoom => apply_zoom_fields(&mut model, user_info),
    }

    model
}

fn apply_google_fields(model: &mut OauthConnectionModel, user_info: UserInfo) {
    model.external_email = Some(user_info.email);
}

fn apply_zoom_fields(model: &mut OauthConnectionModel, user_info: UserInfo) {
    model.external_account_id = Some(user_info.id);
    model.external_email = Some(user_info.email);
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

    fn test_model() -> OauthConnectionModel {
        let now = chrono::Utc::now();
        OauthConnectionModel {
            id: Id::new_v4(),
            user_id: Id::new_v4(),
            provider: MeetingProvider::Google,
            external_account_id: None,
            external_email: Some("coach@example.com".to_string()),
            access_token: "encrypted-access".to_string(),
            refresh_token: Some("encrypted-refresh".to_string()),
            token_expires_at: Some(now.into()),
            token_type: "Bearer".to_string(),
            scopes: "openid email".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    /// 32 bytes hex, matching what `encryption::encrypt` expects.
    const TEST_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// An unusable revoke path must never strand the user in a connected-but-broken state.
    #[tokio::test]
    async fn delete_removes_the_connection_even_when_revocation_cannot_run() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        // Neither stored token is valid ciphertext, so revocation bails before any network call
        // regardless of what the ambient environment configures.
        delete_by_user_and_provider(
            &db,
            &Config::default(),
            model.user_id,
            MeetingProvider::Google,
        )
        .await
    }

    #[test]
    fn revocable_tokens_prefers_the_refresh_token() {
        let model = OauthConnectionModel {
            access_token: encryption::encrypt("access", TEST_KEY).unwrap(),
            refresh_token: Some(encryption::encrypt("refresh", TEST_KEY).unwrap()),
            ..test_model()
        };

        assert_eq!(
            revocable_tokens(&model, TEST_KEY),
            vec!["refresh", "access"]
        );
    }

    #[test]
    fn revocable_tokens_falls_back_to_the_access_token() {
        let model = OauthConnectionModel {
            access_token: encryption::encrypt("access", TEST_KEY).unwrap(),
            refresh_token: None,
            ..test_model()
        };

        assert_eq!(revocable_tokens(&model, TEST_KEY), vec!["access"]);
    }

    #[test]
    fn revocable_tokens_skips_what_it_cannot_decrypt() {
        let model = OauthConnectionModel {
            access_token: encryption::encrypt("access", TEST_KEY).unwrap(),
            refresh_token: Some("not-ciphertext".to_string()),
            ..test_model()
        };

        assert_eq!(revocable_tokens(&model, TEST_KEY), vec!["access"]);
    }

    #[test]
    fn revocable_tokens_is_empty_when_nothing_decrypts() {
        assert!(revocable_tokens(&test_model(), TEST_KEY).is_empty());
    }

    #[tokio::test]
    async fn delete_errors_when_the_user_has_no_connection_for_the_provider() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<OauthConnectionModel, Vec<OauthConnectionModel>, _>(vec![
                vec![],
            ])
            .into_connection();

        let result = delete_by_user_and_provider(
            &db,
            &Config::default(),
            Id::new_v4(),
            MeetingProvider::Google,
        )
        .await;

        assert!(result.is_err());
    }
}
