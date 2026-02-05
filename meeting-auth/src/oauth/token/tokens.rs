//! OAuth token types.

use chrono::{DateTime, Utc};
use secrecy::SecretString;

/// OAuth tokens with metadata.
#[derive(Debug, Clone)]
pub struct Tokens {
    /// Access token for API requests.
    pub access_token: SecretString,
    /// Refresh token for obtaining new access tokens.
    pub refresh_token: Option<SecretString>,
    /// When the access token expires.
    pub expires_at: Option<DateTime<Utc>>,
    /// Token type (usually "Bearer").
    pub token_type: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
}

impl Tokens {
    /// Check if the access token is expired or about to expire soon.
    ///
    /// Returns true if token is expired or will expire within 5 minutes.
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|expires| {
                let now = Utc::now();
                let buffer = chrono::Duration::minutes(5);
                expires <= (now + buffer)
            })
            .unwrap_or(false)
    }

    /// Get the remaining time until expiration.
    pub fn time_until_expiry(&self) -> Option<chrono::Duration> {
        self.expires_at.map(|expires| expires - Utc::now())
    }
}

/// Result of a token refresh operation.
#[derive(Debug, Clone)]
pub struct RefreshResult {
    /// The new tokens.
    pub tokens: Tokens,
    /// True if the refresh token was rotated (Zoom behavior).
    pub refresh_token_rotated: bool,
}

impl RefreshResult {
    /// Create a refresh result with no rotation.
    pub fn no_rotation(tokens: Tokens) -> Self {
        Self {
            tokens,
            refresh_token_rotated: false,
        }
    }

    /// Create a refresh result with rotation.
    pub fn with_rotation(tokens: Tokens) -> Self {
        Self {
            tokens,
            refresh_token_rotated: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_token_not_expired() {
        let tokens = Tokens {
            access_token: SecretString::from("test".to_string()),
            refresh_token: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
            token_type: "Bearer".to_string(),
            scopes: vec![],
        };

        assert!(!tokens.is_expired());
    }

    #[test]
    fn test_token_expired() {
        let tokens = Tokens {
            access_token: SecretString::from("test".to_string()),
            refresh_token: None,
            expires_at: Some(Utc::now() - Duration::hours(1)),
            token_type: "Bearer".to_string(),
            scopes: vec![],
        };

        assert!(tokens.is_expired());
    }

    #[test]
    fn test_token_expiring_soon() {
        let tokens = Tokens {
            access_token: SecretString::from("test".to_string()),
            refresh_token: None,
            expires_at: Some(Utc::now() + Duration::minutes(3)),
            token_type: "Bearer".to_string(),
            scopes: vec![],
        };

        assert!(tokens.is_expired());
    }
}
