//! Token manager with per-user refresh locking.

use std::sync::Arc;

use dashmap::DashMap;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Mutex;
use tracing::debug;

use super::{Storage, Tokens};
use crate::error::{token_error, Error, TokenErrorKind};
use crate::oauth::Provider;

/// Token manager that coordinates token retrieval and refresh with per-user locking.
///
/// The per-user locking prevents race conditions when multiple concurrent requests
/// for the same user trigger token refreshes. Without locking, both requests would
/// try to refresh, one would succeed, and the other would fail with an invalid refresh token.
pub struct Manager<S: Storage> {
    storage: S,
    refresh_locks: DashMap<String, Arc<Mutex<()>>>,
}

impl<S: Storage> Manager<S> {
    /// Create a new token manager with the given storage backend.
    pub fn new(storage: S) -> Self {
        Self {
            storage,
            refresh_locks: DashMap::new(),
        }
    }

    /// Get a valid access token for a user, refreshing if needed.
    ///
    /// This method:
    /// 1. Retrieves the stored tokens
    /// 2. Checks if the access token is expired
    /// 3. If expired, refreshes the token (with locking to prevent races)
    /// 4. Returns the valid access token
    ///
    /// # Arguments
    ///
    /// * `provider` - The OAuth provider implementation
    /// * `user_id` - Unique user identifier
    ///
    /// # Returns
    ///
    /// A valid access token, or an error if refresh fails.
    pub async fn get_valid_token<P: Provider>(
        &self,
        provider: &P,
        user_id: &str,
    ) -> Result<SecretString, Error> {
        let provider_id = provider.provider().as_str();

        // Get stored tokens
        let tokens = self
            .storage
            .get(user_id, provider_id)
            .await?
            .ok_or_else(|| token_error(TokenErrorKind::NotFound, "No tokens found for user"))?;

        // Check if token is expired
        if !tokens.is_expired() {
            return Ok(tokens.access_token);
        }

        // Token is expired, need to refresh
        debug!("Token expired for user {}, refreshing", user_id);

        // Get or create a lock for this user
        let lock = self
            .refresh_locks
            .entry(user_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();

        // Acquire the lock for this user
        let _guard = lock.lock().await;

        // Double-check if token is still expired (another request might have refreshed it)
        let tokens = self
            .storage
            .get(user_id, provider_id)
            .await?
            .ok_or_else(|| token_error(TokenErrorKind::NotFound, "Tokens disappeared during refresh"))?;

        if !tokens.is_expired() {
            debug!("Token was refreshed by another request");
            return Ok(tokens.access_token);
        }

        // Refresh the token
        let refresh_token = tokens
            .refresh_token
            .as_ref()
            .ok_or_else(|| token_error(TokenErrorKind::Refresh, "No refresh token available"))?;

        let refresh_result = provider
            .refresh_token(refresh_token.expose_secret())
            .await
            .map_err(|e| token_error(TokenErrorKind::Refresh, &format!("Token refresh failed: {}", e)))?;

        // Store the new tokens
        if refresh_result.refresh_token_rotated {
            // For rotating refresh tokens (Zoom), use atomic update
            debug!("Using atomic update for rotating refresh token");
            self.storage
                .update_atomic(
                    user_id,
                    provider_id,
                    Some(refresh_token.expose_secret()),
                    refresh_result.tokens.clone(),
                )
                .await?;
        } else {
            // For non-rotating refresh tokens, regular store is fine
            self.storage
                .store(user_id, provider_id, refresh_result.tokens.clone())
                .await?;
        }

        debug!("Token refreshed successfully for user {}", user_id);

        Ok(refresh_result.tokens.access_token)
    }

    /// Store tokens for a user.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Unique user identifier
    /// * `provider_id` - Provider identifier
    /// * `tokens` - The tokens to store
    pub async fn store_tokens(
        &self,
        user_id: &str,
        provider_id: &str,
        tokens: Tokens,
    ) -> Result<(), Error> {
        self.storage.store(user_id, provider_id, tokens).await
    }

    /// Delete tokens for a user.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Unique user identifier
    /// * `provider_id` - Provider identifier
    pub async fn delete_tokens(&self, user_id: &str, provider_id: &str) -> Result<(), Error> {
        self.storage.delete(user_id, provider_id).await
    }

    /// Get stored tokens for a user (may be expired).
    ///
    /// # Arguments
    ///
    /// * `user_id` - Unique user identifier
    /// * `provider_id` - Provider identifier
    pub async fn get_tokens(&self, user_id: &str, provider_id: &str) -> Result<Option<Tokens>, Error> {
        self.storage.get(user_id, provider_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::Mutex as TokioMutex;

    // Mock storage for testing
    struct MockStorage {
        tokens: Arc<TokioMutex<HashMap<String, Tokens>>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                tokens: Arc::new(TokioMutex::new(HashMap::new())),
            }
        }

        fn key(user_id: &str, provider_id: &str) -> String {
            format!("{}:{}", user_id, provider_id)
        }
    }

    #[async_trait]
    impl Storage for MockStorage {
        async fn store(&self, user_id: &str, provider_id: &str, tokens: Tokens) -> Result<(), Error> {
            let mut map = self.tokens.lock().await;
            map.insert(Self::key(user_id, provider_id), tokens);
            Ok(())
        }

        async fn get(&self, user_id: &str, provider_id: &str) -> Result<Option<Tokens>, Error> {
            let map = self.tokens.lock().await;
            Ok(map.get(&Self::key(user_id, provider_id)).cloned())
        }

        async fn update_atomic(
            &self,
            user_id: &str,
            provider_id: &str,
            _old_refresh: Option<&str>,
            new_tokens: Tokens,
        ) -> Result<(), Error> {
            self.store(user_id, provider_id, new_tokens).await
        }

        async fn delete(&self, user_id: &str, provider_id: &str) -> Result<(), Error> {
            let mut map = self.tokens.lock().await;
            map.remove(&Self::key(user_id, provider_id));
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_store_and_get_tokens() {
        let storage = MockStorage::new();
        let manager = Manager::new(storage);

        let tokens = Tokens {
            access_token: SecretString::from("access".to_string()),
            refresh_token: Some(SecretString::from("refresh".to_string())),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".to_string(),
            scopes: vec![],
        };

        manager
            .store_tokens("user1", "google", tokens.clone())
            .await
            .unwrap();

        let retrieved = manager.get_tokens("user1", "google").await.unwrap();
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_delete_tokens() {
        let storage = MockStorage::new();
        let manager = Manager::new(storage);

        let tokens = Tokens {
            access_token: SecretString::from("access".to_string()),
            refresh_token: Some(SecretString::from("refresh".to_string())),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".to_string(),
            scopes: vec![],
        };

        manager
            .store_tokens("user1", "google", tokens)
            .await
            .unwrap();

        manager.delete_tokens("user1", "google").await.unwrap();

        let retrieved = manager.get_tokens("user1", "google").await.unwrap();
        assert!(retrieved.is_none());
    }
}
