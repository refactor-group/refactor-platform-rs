//! AES-256-GCM encryption utilities for securing tokens stored at rest.
//!
//! Provides functions to encrypt and decrypt sensitive token data before storing
//! in a database. The encryption key should be a 32-byte key provided as a
//! hex-encoded string (64 characters).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::Rng;

use crate::error::{Error, ErrorKind, StorageErrorKind};

/// 12-byte nonce size for AES-GCM
const NONCE_SIZE: usize = 12;

fn encryption_err() -> Error {
    Error {
        source: None,
        error_kind: ErrorKind::Storage(StorageErrorKind::EncryptionFailed),
    }
}

fn decryption_err() -> Error {
    Error {
        source: None,
        error_kind: ErrorKind::Storage(StorageErrorKind::DecryptionFailed),
    }
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
pub fn encrypt(plaintext: &str, key_hex: &str) -> Result<String, Error> {
    let key = parse_key(key_hex)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| encryption_err())?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| encryption_err())?;

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
pub fn decrypt(ciphertext_b64: &str, key_hex: &str) -> Result<String, Error> {
    let key = parse_key(key_hex)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| decryption_err())?;

    let combined = BASE64.decode(ciphertext_b64).map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: ErrorKind::Storage(StorageErrorKind::DecryptionFailed),
    })?;

    if combined.len() < NONCE_SIZE {
        return Err(decryption_err());
    }

    let (nonce_bytes, ciphertext) = combined.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext_bytes = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| decryption_err())?;

    String::from_utf8(plaintext_bytes).map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: ErrorKind::Storage(StorageErrorKind::DecryptionFailed),
    })
}

/// Encrypts a value when an encryption key is available.
///
/// Returns `Err` with `StorageErrorKind::EncryptionFailed` if a plaintext value
/// is provided but no key is configured.
pub fn encrypt_optional(
    plaintext: Option<&str>,
    key_hex: Option<&str>,
) -> Result<Option<String>, Error> {
    match (plaintext, key_hex) {
        (Some(pt), Some(key)) => Ok(Some(encrypt(pt, key)?)),
        (Some(_), None) => Err(encryption_err()),
        (None, _) => Ok(None),
    }
}

/// Decrypts a value when an encryption key is available.
///
/// Returns `Err` with `StorageErrorKind::DecryptionFailed` if a ciphertext value
/// is provided but no key is configured.
pub fn decrypt_optional(
    ciphertext: Option<&str>,
    key_hex: Option<&str>,
) -> Result<Option<String>, Error> {
    match (ciphertext, key_hex) {
        (Some(ct), Some(key)) => Ok(Some(decrypt(ct, key)?)),
        (Some(_), None) => Err(decryption_err()),
        (None, _) => Ok(None),
    }
}

fn parse_key(key_hex: &str) -> Result<[u8; 32], Error> {
    let bytes = hex::decode(key_hex).map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: ErrorKind::Storage(StorageErrorKind::EncryptionFailed),
    })?;
    if bytes.len() != 32 {
        return Err(encryption_err());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::StorageErrorKind;

    const TEST_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = "my-secret-api-key-12345";
        let encrypted = encrypt(plaintext, TEST_KEY).expect("encryption should succeed");
        assert_ne!(encrypted, plaintext);
        let decrypted = decrypt(&encrypted, TEST_KEY).expect("decryption should succeed");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_produces_different_outputs() {
        let plaintext = "test-api-key";
        let encrypted1 = encrypt(plaintext, TEST_KEY).unwrap();
        let encrypted2 = encrypt(plaintext, TEST_KEY).unwrap();
        assert_ne!(encrypted1, encrypted2);
        assert_eq!(decrypt(&encrypted1, TEST_KEY).unwrap(), plaintext);
        assert_eq!(decrypt(&encrypted2, TEST_KEY).unwrap(), plaintext);
    }

    #[test]
    fn test_invalid_key_returns_encryption_failed() {
        let result = encrypt("test", "not-valid-hex!");
        assert!(matches!(
            result,
            Err(Error {
                error_kind: ErrorKind::Storage(StorageErrorKind::EncryptionFailed),
                ..
            })
        ));
    }

    #[test]
    fn test_wrong_key_returns_decryption_failed() {
        let encrypted = encrypt("secret", TEST_KEY).unwrap();
        let wrong_key = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let result = decrypt(&encrypted, wrong_key);
        assert!(matches!(
            result,
            Err(Error {
                error_kind: ErrorKind::Storage(StorageErrorKind::DecryptionFailed),
                ..
            })
        ));
    }

    #[test]
    fn test_corrupted_ciphertext_returns_decryption_failed() {
        let result = decrypt("not_valid_base64!!!", TEST_KEY);
        assert!(matches!(
            result,
            Err(Error {
                error_kind: ErrorKind::Storage(StorageErrorKind::DecryptionFailed),
                ..
            })
        ));
    }

    #[test]
    fn test_ciphertext_too_short_returns_decryption_failed() {
        let result = decrypt("YWJj", TEST_KEY); // "abc" in base64
        assert!(matches!(
            result,
            Err(Error {
                error_kind: ErrorKind::Storage(StorageErrorKind::DecryptionFailed),
                ..
            })
        ));
    }

    #[test]
    fn test_encrypt_optional_with_key() {
        let result = encrypt_optional(Some("test"), Some(TEST_KEY));
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_encrypt_optional_without_key_returns_encryption_failed() {
        let result = encrypt_optional(Some("test"), None);
        assert!(matches!(
            result,
            Err(Error {
                error_kind: ErrorKind::Storage(StorageErrorKind::EncryptionFailed),
                ..
            })
        ));
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
