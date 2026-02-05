//! PKCE (Proof Key for Code Exchange) support for OAuth 2.0.
//!
//! Implements RFC 7636 for securing authorization code flows in public clients.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use sha2::{Digest, Sha256};

/// PKCE code verifier (random string).
#[derive(Debug, Clone)]
pub struct PkceVerifier(String);

impl PkceVerifier {
    /// Generate a new random PKCE verifier.
    ///
    /// Creates a cryptographically random string of 43-128 characters.
    pub fn generate() -> Self {
        let random_bytes: [u8; 32] = rand::thread_rng().gen();
        let verifier = URL_SAFE_NO_PAD.encode(random_bytes);
        Self(verifier)
    }

    /// Create a PKCE verifier from an existing string.
    pub fn from_string(verifier: String) -> Self {
        Self(verifier)
    }

    /// Get the verifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Generate the corresponding code challenge.
    pub fn challenge(&self) -> PkceChallenge {
        PkceChallenge::from_verifier(self)
    }
}

/// PKCE code challenge (SHA256 hash of verifier).
#[derive(Debug, Clone)]
pub struct PkceChallenge(String);

impl PkceChallenge {
    /// Create a code challenge from a verifier.
    ///
    /// Uses SHA256 hashing and base64url encoding as per RFC 7636.
    pub fn from_verifier(verifier: &PkceVerifier) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_str().as_bytes());
        let hash = hasher.finalize();
        let challenge = URL_SAFE_NO_PAD.encode(hash);
        Self(challenge)
    }

    /// Get the challenge string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_verifier_generation() {
        let verifier = PkceVerifier::generate();
        assert!(!verifier.as_str().is_empty());
        assert!(verifier.as_str().len() >= 43);
    }

    #[test]
    fn test_pkce_challenge_generation() {
        let verifier = PkceVerifier::from_string("test_verifier".to_string());
        let challenge = verifier.challenge();
        assert!(!challenge.as_str().is_empty());
    }

    #[test]
    fn test_pkce_challenge_deterministic() {
        let verifier = PkceVerifier::from_string("test_verifier".to_string());
        let challenge1 = verifier.challenge();
        let challenge2 = verifier.challenge();
        assert_eq!(challenge1.as_str(), challenge2.as_str());
    }
}
