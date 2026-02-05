//! HMAC-SHA256 webhook signature validation.

use std::collections::HashMap;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::WebhookValidator;
use crate::error::{webhook_error, Error, WebhookErrorKind};

type HmacSha256 = Hmac<Sha256>;

/// HMAC-SHA256 webhook validator.
///
/// Validates webhook signatures using HMAC-SHA256, as used by Recall.ai, AssemblyAI, and others.
pub struct HmacWebhookValidator {
    provider_id: String,
    secret: String,
    signature_header: String,
}

impl HmacWebhookValidator {
    /// Create a new HMAC webhook validator.
    ///
    /// # Arguments
    ///
    /// * `provider_id` - Provider identifier
    /// * `secret` - Webhook signing secret
    /// * `signature_header` - Name of the header containing the signature
    pub fn new(provider_id: String, secret: String, signature_header: String) -> Self {
        Self {
            provider_id,
            secret,
            signature_header,
        }
    }
}

impl WebhookValidator for HmacWebhookValidator {
    fn validate(&self, headers: &HashMap<String, String>, body: &[u8]) -> Result<bool, Error> {
        // Get the signature from headers
        let signature = headers
            .get(&self.signature_header)
            .ok_or_else(|| {
                webhook_error(
                    WebhookErrorKind::MissingSignature,
                    &format!("Missing signature header: {}", self.signature_header),
                )
            })?;

        // Parse the hex-encoded signature
        let expected_sig = hex::decode(signature.trim_start_matches("sha256="))
            .map_err(|_| {
                webhook_error(
                    WebhookErrorKind::InvalidSignature,
                    "Invalid signature format",
                )
            })?;

        // Compute HMAC
        let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes())
            .map_err(|_| {
                webhook_error(
                    WebhookErrorKind::InvalidPayload,
                    "Invalid HMAC key",
                )
            })?;
        mac.update(body);

        // Verify the signature
        mac.verify_slice(&expected_sig)
            .map(|_| true)
            .or(Ok(false))
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_signature() {
        let secret = "test_secret";
        let body = b"test payload";

        // Compute expected signature
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());

        let validator = HmacWebhookValidator::new(
            "test_provider".to_string(),
            secret.to_string(),
            "X-Webhook-Signature".to_string(),
        );

        let mut headers = HashMap::new();
        headers.insert("X-Webhook-Signature".to_string(), signature);

        assert!(validator.validate(&headers, body).unwrap());
    }

    #[test]
    fn test_invalid_signature() {
        let validator = HmacWebhookValidator::new(
            "test_provider".to_string(),
            "test_secret".to_string(),
            "X-Webhook-Signature".to_string(),
        );

        let mut headers = HashMap::new();
        headers.insert("X-Webhook-Signature".to_string(), "invalid".to_string());

        let result = validator.validate(&headers, b"test payload");
        assert!(result.is_err());
    }
}
