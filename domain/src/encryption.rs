//! AES-256-GCM encryption utilities for securing API keys stored in the database.
//!
//! This module provides functions to encrypt and decrypt sensitive data like API keys
//! before storing them in the database. The encryption key should be a 32-byte key
//! provided via the ENCRYPTION_KEY environment variable (hex-encoded).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::Rng;
use thiserror::Error;

/// 12-byte nonce size for AES-GCM
const NONCE_SIZE: usize = 12;

/// Errors that can occur during encryption/decryption operations
#[derive(Debug, Error)]
pub enum EncryptionError {
    #[error("Invalid encryption key: must be 32 bytes (64 hex characters)")]
    InvalidKey,

    #[error("Failed to decode hex key: {0}")]
    HexDecodeError(#[from] hex::FromHexError),

    #[error("Failed to decode base64 ciphertext: {0}")]
    Base64DecodeError(#[from] base64::DecodeError),

    #[error("Encryption failed")]
    EncryptionFailed,

    #[error("Decryption failed - data may be corrupted or key is incorrect")]
    DecryptionFailed,

    #[error("Ciphertext too short - missing nonce")]
    CiphertextTooShort,

    #[error("No encryption key configured")]
    NoKeyConfigured,
}

/// Encrypts plaintext using AES-256-GCM with a random nonce.
///
/// The nonce is prepended to the ciphertext, and the result is base64-encoded
/// for safe storage in a text database column.
///
/// # Arguments
/// * `plaintext` - The data to encrypt
/// * `key_hex` - The 32-byte encryption key as a hex string (64 characters)
///
/// # Returns
/// Base64-encoded string containing nonce + ciphertext
pub fn encrypt(plaintext: &str, key_hex: &str) -> Result<String, EncryptionError> {
    let key = parse_key(key_hex)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| EncryptionError::InvalidKey)?;

    // Generate a random 12-byte nonce
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encrypt the plaintext
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| EncryptionError::EncryptionFailed)?;

    // Prepend nonce to ciphertext and base64 encode
    let mut combined = nonce_bytes.to_vec();
    combined.extend(ciphertext);

    Ok(BASE64.encode(combined))
}

/// Decrypts a base64-encoded ciphertext that was encrypted with `encrypt()`.
///
/// # Arguments
/// * `ciphertext_b64` - Base64-encoded string containing nonce + ciphertext
/// * `key_hex` - The 32-byte encryption key as a hex string (64 characters)
///
/// # Returns
/// The original plaintext string
pub fn decrypt(ciphertext_b64: &str, key_hex: &str) -> Result<String, EncryptionError> {
    let key = parse_key(key_hex)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| EncryptionError::InvalidKey)?;

    // Decode base64
    let combined = BASE64.decode(ciphertext_b64)?;

    // Split nonce and ciphertext
    if combined.len() < NONCE_SIZE {
        return Err(EncryptionError::CiphertextTooShort);
    }

    let (nonce_bytes, ciphertext) = combined.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    // Decrypt
    let plaintext_bytes = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| EncryptionError::DecryptionFailed)?;

    String::from_utf8(plaintext_bytes).map_err(|_| EncryptionError::DecryptionFailed)
}

/// Encrypts a value if an encryption key is available, otherwise returns None.
///
/// This is useful for optional encryption when the key might not be configured.
pub fn encrypt_optional(
    plaintext: Option<&str>,
    key_hex: Option<&str>,
) -> Result<Option<String>, EncryptionError> {
    match (plaintext, key_hex) {
        (Some(pt), Some(key)) => Ok(Some(encrypt(pt, key)?)),
        (Some(_), None) => Err(EncryptionError::NoKeyConfigured),
        (None, _) => Ok(None),
    }
}

/// Decrypts a value if an encryption key is available, otherwise returns None.
pub fn decrypt_optional(
    ciphertext: Option<&str>,
    key_hex: Option<&str>,
) -> Result<Option<String>, EncryptionError> {
    match (ciphertext, key_hex) {
        (Some(ct), Some(key)) => Ok(Some(decrypt(ct, key)?)),
        (Some(_), None) => Err(EncryptionError::NoKeyConfigured),
        (None, _) => Ok(None),
    }
}

/// Parses a hex-encoded 32-byte key
fn parse_key(key_hex: &str) -> Result<[u8; 32], EncryptionError> {
    let bytes = hex::decode(key_hex)?;
    if bytes.len() != 32 {
        return Err(EncryptionError::InvalidKey);
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test key: 32 bytes = 64 hex characters
    const TEST_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = "my-secret-api-key-12345";
        let encrypted = encrypt(plaintext, TEST_KEY).expect("encryption should succeed");

        // Encrypted should be different from plaintext
        assert_ne!(encrypted, plaintext);

        // Should be able to decrypt back to original
        let decrypted = decrypt(&encrypted, TEST_KEY).expect("decryption should succeed");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_produces_different_outputs() {
        // Due to random nonce, encrypting same plaintext should produce different ciphertexts
        let plaintext = "test-api-key";
        let encrypted1 = encrypt(plaintext, TEST_KEY).unwrap();
        let encrypted2 = encrypt(plaintext, TEST_KEY).unwrap();

        assert_ne!(encrypted1, encrypted2);

        // But both should decrypt to the same value
        assert_eq!(decrypt(&encrypted1, TEST_KEY).unwrap(), plaintext);
        assert_eq!(decrypt(&encrypted2, TEST_KEY).unwrap(), plaintext);
    }

    #[test]
    fn test_invalid_key_length() {
        let result = encrypt("test", "short_key");
        assert!(matches!(
            result,
            Err(EncryptionError::HexDecodeError(_)) | Err(EncryptionError::InvalidKey)
        ));
    }

    #[test]
    fn test_wrong_key_fails_decryption() {
        let plaintext = "secret";
        let encrypted = encrypt(plaintext, TEST_KEY).unwrap();

        let wrong_key = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let result = decrypt(&encrypted, wrong_key);

        assert!(matches!(result, Err(EncryptionError::DecryptionFailed)));
    }

    #[test]
    fn test_corrupted_ciphertext_fails() {
        let result = decrypt("not_valid_base64!!!", TEST_KEY);
        assert!(matches!(result, Err(EncryptionError::Base64DecodeError(_))));
    }

    #[test]
    fn test_ciphertext_too_short() {
        // Valid base64 but too short to contain nonce
        let result = decrypt("YWJj", TEST_KEY); // "abc" in base64
        assert!(matches!(result, Err(EncryptionError::CiphertextTooShort)));
    }

    #[test]
    fn test_encrypt_optional_with_key() {
        let result = encrypt_optional(Some("test"), Some(TEST_KEY));
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_encrypt_optional_without_key() {
        let result = encrypt_optional(Some("test"), None);
        assert!(matches!(result, Err(EncryptionError::NoKeyConfigured)));
    }

    #[test]
    fn test_encrypt_optional_without_value() {
        let result = encrypt_optional(None, Some(TEST_KEY));
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_unicode_plaintext() {
        let plaintext = "API密钥🔐with-unicode-✓";
        let encrypted = encrypt(plaintext, TEST_KEY).unwrap();
        let decrypted = decrypt(&encrypted, TEST_KEY).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_empty_plaintext() {
        let plaintext = "";
        let encrypted = encrypt(plaintext, TEST_KEY).unwrap();
        let decrypted = decrypt(&encrypted, TEST_KEY).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
