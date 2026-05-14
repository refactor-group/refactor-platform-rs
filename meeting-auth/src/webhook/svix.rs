//! Svix HMAC-SHA256 webhook signature validation (used by Recall.ai).
//!
//! Svix signing format:
//! - Signed content: `{svix-id}.{svix-timestamp}.{raw-body}`
//! - Secret format: `whsec_<base64url-encoded-key>` (strip prefix then base64-decode)
//! - Signature header: `svix-signature` — space-delimited list of `v1,<base64-sig>` entries
//! - Replay protection: reject requests with `svix-timestamp` older than 5 minutes

use std::collections::HashMap;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::{webhook_error, Error, WebhookErrorKind};

type HmacSha256 = Hmac<Sha256>;

const MAX_TIMESTAMP_AGE_SECS: i64 = 300; // 5 minutes past — replay protection
const MAX_TIMESTAMP_FUTURE_SECS: i64 = 60; // 1 minute future — clock skew tolerance
const SVIX_SECRET_PREFIX: &str = "whsec_";

/// Svix webhook validator for Recall.ai events.
pub struct Validator {
    provider_id: String,
    /// Decoded HMAC key (bytes extracted from the `whsec_...` secret).
    secret_bytes: Vec<u8>,
}

impl Validator {
    /// Create a new Svix validator.
    ///
    /// # Arguments
    ///
    /// * `provider_id` - Provider identifier (e.g. `"recall_ai"`)
    /// * `secret` - Webhook signing secret in `whsec_<base64>` format
    pub fn new(provider_id: String, secret: &str) -> Result<Self, Error> {
        let b64 = secret.strip_prefix(SVIX_SECRET_PREFIX).ok_or_else(|| {
            webhook_error(
                WebhookErrorKind::InvalidSignature,
                "Svix secret must start with 'whsec_'",
            )
        })?;

        let secret_bytes = BASE64.decode(b64).map_err(|_| {
            webhook_error(
                WebhookErrorKind::InvalidSignature,
                "Failed to base64-decode Svix webhook secret",
            )
        })?;

        Ok(Self {
            provider_id,
            secret_bytes,
        })
    }
}

impl crate::webhook::Validator for Validator {
    fn validate(&self, headers: &HashMap<String, String>, body: &[u8]) -> Result<bool, Error> {
        // Recall.ai workspaces created after 2025-12-15 send "webhook-*" headers instead of "svix-*".
        let svix_id = headers
            .get("svix-id")
            .or_else(|| headers.get("webhook-id"))
            .ok_or_else(|| {
                webhook_error(
                    WebhookErrorKind::MissingSignature,
                    "Missing svix-id/webhook-id header",
                )
            })?;

        let svix_timestamp = headers
            .get("svix-timestamp")
            .or_else(|| headers.get("webhook-timestamp"))
            .ok_or_else(|| {
                webhook_error(
                    WebhookErrorKind::MissingSignature,
                    "Missing svix-timestamp/webhook-timestamp header",
                )
            })?;

        let svix_signature = headers
            .get("svix-signature")
            .or_else(|| headers.get("webhook-signature"))
            .ok_or_else(|| {
                webhook_error(
                    WebhookErrorKind::MissingSignature,
                    "Missing svix-signature/webhook-signature header",
                )
            })?;

        // Replay protection: reject timestamps older than 5 minutes or more than 1 minute
        // in the future. Asymmetric bounds prevent future-dating from widening the replay window.
        let timestamp: i64 = svix_timestamp.parse().map_err(|_| {
            webhook_error(
                WebhookErrorKind::InvalidPayload,
                "svix-timestamp is not a valid integer",
            )
        })?;
        let now = chrono::Utc::now().timestamp();
        let age = now - timestamp; // positive = past, negative = future
        if !(-MAX_TIMESTAMP_FUTURE_SECS..=MAX_TIMESTAMP_AGE_SECS).contains(&age) {
            return Err(webhook_error(
                WebhookErrorKind::TimestampExpired,
                &format!(
                    "svix-timestamp {} is outside the allowed window (now={})",
                    timestamp, now
                ),
            ));
        }

        // Build signed content: "{svix-id}.{svix-timestamp}.{body}"
        let signed_content = format!("{}.{}.", svix_id, svix_timestamp);
        let mut signed_bytes = signed_content.into_bytes();
        signed_bytes.extend_from_slice(body);

        // Compute expected HMAC
        let mut mac = HmacSha256::new_from_slice(&self.secret_bytes).map_err(|_| {
            webhook_error(WebhookErrorKind::InvalidPayload, "Invalid HMAC key length")
        })?;
        mac.update(&signed_bytes);
        let expected = mac.finalize().into_bytes();

        // Verify against each `v1,<base64-sig>` in the space-delimited header
        // Only process v1 entries; skip unknown versions (e.g. v2) rather than
        // attempting to decode them with the wrong algorithm.
        for entry in svix_signature.split_whitespace() {
            let Some(b64_sig) = entry.strip_prefix("v1,") else {
                continue;
            };
            if let Ok(sig_bytes) = BASE64.decode(b64_sig) {
                if constant_time_eq(&expected, &sig_bytes) {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

/// Constant-time byte slice comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webhook::Validator as _;

    fn make_secret_and_validator() -> (Vec<u8>, Validator) {
        let raw_key = b"test_secret_key_32bytes_long_pad";
        let b64_key = BASE64.encode(raw_key);
        let secret = format!("whsec_{}", b64_key);
        let validator = Validator::new("recall_ai".to_string(), &secret).unwrap();
        (raw_key.to_vec(), validator)
    }

    fn sign(key: &[u8], svix_id: &str, timestamp: i64, body: &[u8]) -> String {
        let signed_content = format!("{}.{}.", svix_id, timestamp);
        let mut signed_bytes = signed_content.into_bytes();
        signed_bytes.extend_from_slice(body);

        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(&signed_bytes);
        let sig = mac.finalize().into_bytes();
        format!("v1,{}", BASE64.encode(sig))
    }

    #[test]
    fn valid_signature_returns_true() {
        let (key, validator) = make_secret_and_validator();
        let body = b"{\"event\":\"bot.status_change\"}";
        let svix_id = "msg_abc123";
        let timestamp = chrono::Utc::now().timestamp();
        let sig = sign(&key, svix_id, timestamp, body);

        let mut headers = HashMap::new();
        headers.insert("svix-id".to_string(), svix_id.to_string());
        headers.insert("svix-timestamp".to_string(), timestamp.to_string());
        headers.insert("svix-signature".to_string(), sig);

        assert!(validator.validate(&headers, body).unwrap());
    }

    #[test]
    fn invalid_signature_returns_false() {
        let (_, validator) = make_secret_and_validator();
        let body = b"{\"event\":\"bot.status_change\"}";
        let timestamp = chrono::Utc::now().timestamp();

        let mut headers = HashMap::new();
        headers.insert("svix-id".to_string(), "msg_abc123".to_string());
        headers.insert("svix-timestamp".to_string(), timestamp.to_string());
        headers.insert(
            "svix-signature".to_string(),
            "v1,invalidsignature==".to_string(),
        );

        assert!(!validator.validate(&headers, body).unwrap());
    }

    #[test]
    fn expired_timestamp_returns_error() {
        let (key, validator) = make_secret_and_validator();
        let body = b"test";
        let svix_id = "msg_old";
        let old_timestamp = chrono::Utc::now().timestamp() - 400; // > 5 minutes ago
        let sig = sign(&key, svix_id, old_timestamp, body);

        let mut headers = HashMap::new();
        headers.insert("svix-id".to_string(), svix_id.to_string());
        headers.insert("svix-timestamp".to_string(), old_timestamp.to_string());
        headers.insert("svix-signature".to_string(), sig);

        let result = validator.validate(&headers, body);
        assert!(result.is_err());
    }

    #[test]
    fn missing_header_returns_error() {
        let (_, validator) = make_secret_and_validator();

        let headers = HashMap::new();
        let result = validator.validate(&headers, b"body");
        assert!(result.is_err());
    }

    #[test]
    fn invalid_secret_prefix_returns_error() {
        let result = Validator::new("recall_ai".to_string(), "invalid_secret");
        assert!(result.is_err());
    }

    #[test]
    fn multi_entry_signature_header_accepts_valid_second_entry() {
        let (key, validator) = make_secret_and_validator();
        let body = b"{\"event\":\"bot.done\"}";
        let svix_id = "msg_multi";
        let timestamp = chrono::Utc::now().timestamp();
        let valid_sig = sign(&key, svix_id, timestamp, body);

        let mut headers = HashMap::new();
        headers.insert("svix-id".to_string(), svix_id.to_string());
        headers.insert("svix-timestamp".to_string(), timestamp.to_string());
        // First entry is bogus; second entry is the correct signature.
        headers.insert(
            "svix-signature".to_string(),
            format!("v1,invalidbase64garbage {}", valid_sig),
        );

        assert!(validator.validate(&headers, body).unwrap());
    }

    #[test]
    fn webhook_alias_headers_validate_correctly() {
        let (key, validator) = make_secret_and_validator();
        let body = b"{\"event\":\"recording.done\"}";
        let svix_id = "wh_alias_test";
        let timestamp = chrono::Utc::now().timestamp();
        let sig = sign(&key, svix_id, timestamp, body);

        let mut headers = HashMap::new();
        headers.insert("webhook-id".to_string(), svix_id.to_string());
        headers.insert("webhook-timestamp".to_string(), timestamp.to_string());
        headers.insert("webhook-signature".to_string(), sig);

        assert!(validator.validate(&headers, body).unwrap());
    }

    #[test]
    fn unknown_version_prefix_is_skipped_and_falls_through_to_false() {
        let (_, validator) = make_secret_and_validator();
        let body = b"test";
        let timestamp = chrono::Utc::now().timestamp();

        let mut headers = HashMap::new();
        headers.insert("svix-id".to_string(), "msg_v2".to_string());
        headers.insert("svix-timestamp".to_string(), timestamp.to_string());
        // Only a v2 entry — the validator must skip it and return false, not error.
        headers.insert("svix-signature".to_string(), "v2,somesig".to_string());

        assert!(!validator.validate(&headers, body).unwrap());
    }
}
