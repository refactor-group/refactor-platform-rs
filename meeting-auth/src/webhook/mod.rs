//! Webhook signature validation.

mod hmac;

pub use hmac::HmacWebhookValidator;

use std::collections::HashMap;

use crate::error::Error;

/// Trait for validating webhook signatures.
pub trait WebhookValidator: Send + Sync {
    /// Validate a webhook request.
    ///
    /// # Arguments
    ///
    /// * `headers` - HTTP headers from the webhook request
    /// * `body` - Raw request body bytes
    ///
    /// # Returns
    ///
    /// `true` if signature is valid, `false` otherwise.
    fn validate(&self, headers: &HashMap<String, String>, body: &[u8]) -> Result<bool, Error>;

    /// Get the provider identifier for this validator.
    fn provider_id(&self) -> &str;
}
