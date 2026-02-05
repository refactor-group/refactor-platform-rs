//! CSRF state management for OAuth flows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use rand::Rng;

/// State data stored during OAuth flow.
#[derive(Debug, Clone)]
pub struct StateData {
    /// PKCE verifier if PKCE was used.
    pub pkce_verifier: Option<String>,
    /// When this state expires.
    pub expires_at: DateTime<Utc>,
    /// Additional metadata.
    pub metadata: HashMap<String, String>,
}

/// Manager for OAuth state parameters with expiration.
///
/// Generates and validates CSRF state tokens to prevent cross-site request forgery attacks.
#[derive(Clone)]
pub struct StateManager {
    states: Arc<Mutex<HashMap<String, StateData>>>,
    ttl: Duration,
}

impl StateManager {
    /// Create a new state manager with default TTL of 10 minutes.
    pub fn new() -> Self {
        Self {
            states: Arc::new(Mutex::new(HashMap::new())),
            ttl: Duration::minutes(10),
        }
    }

    /// Create a new state manager with custom TTL.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            states: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    /// Generate a new state token and store associated data.
    ///
    /// # Arguments
    ///
    /// * `pkce_verifier` - Optional PKCE verifier to store with this state
    /// * `metadata` - Additional metadata to associate with this state
    ///
    /// # Returns
    ///
    /// The generated state token string.
    pub fn generate(
        &self,
        pkce_verifier: Option<String>,
        metadata: HashMap<String, String>,
    ) -> String {
        let state = Self::generate_token();
        let expires_at = Utc::now() + self.ttl;

        let data = StateData {
            pkce_verifier,
            expires_at,
            metadata,
        };

        let mut states = self.states.lock().unwrap();
        states.insert(state.clone(), data);

        state
    }

    /// Validate and consume a state token.
    ///
    /// Removes the state from storage and returns associated data if valid.
    ///
    /// # Arguments
    ///
    /// * `state` - The state token to validate
    ///
    /// # Returns
    ///
    /// `Some(StateData)` if valid, `None` if invalid or expired.
    pub fn validate(&self, state: &str) -> Option<StateData> {
        let mut states = self.states.lock().unwrap();

        // Remove and return the state data if it exists
        if let Some(data) = states.remove(state) {
            // Check if expired
            if Utc::now() > data.expires_at {
                return None;
            }
            Some(data)
        } else {
            None
        }
    }

    /// Clean up expired states.
    ///
    /// Should be called periodically to prevent memory leaks.
    pub fn cleanup_expired(&self) {
        let mut states = self.states.lock().unwrap();
        let now = Utc::now();
        states.retain(|_, data| data.expires_at > now);
    }

    /// Generate a cryptographically random state token.
    fn generate_token() -> String {
        let random_bytes: [u8; 32] = rand::thread_rng().gen();
        hex::encode(random_bytes)
    }
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_state() {
        let manager = StateManager::new();
        let state = manager.generate(None, HashMap::new());
        assert!(!state.is_empty());
        assert_eq!(state.len(), 64); // 32 bytes hex encoded
    }

    #[test]
    fn test_validate_state() {
        let manager = StateManager::new();
        let state = manager.generate(Some("verifier".to_string()), HashMap::new());

        let data = manager.validate(&state);
        assert!(data.is_some());
        assert_eq!(data.unwrap().pkce_verifier, Some("verifier".to_string()));
    }

    #[test]
    fn test_validate_invalid_state() {
        let manager = StateManager::new();
        let data = manager.validate("invalid_state");
        assert!(data.is_none());
    }

    #[test]
    fn test_state_consumed_after_validation() {
        let manager = StateManager::new();
        let state = manager.generate(None, HashMap::new());

        manager.validate(&state);
        let data = manager.validate(&state);
        assert!(data.is_none());
    }

    #[test]
    fn test_expired_state() {
        let manager = StateManager::with_ttl(Duration::seconds(-1));
        let state = manager.generate(None, HashMap::new());

        let data = manager.validate(&state);
        assert!(data.is_none());
    }
}
