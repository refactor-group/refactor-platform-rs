//! Shared input validation for web-layer parameter structs.
//!
//! Validation here is about **HTTP request shape**, not business rules:
//! we reject empty/oversized/obviously-malformed inputs at the boundary
//! before they reach the domain layer. Rejections return `400 Bad Request`
//! (vs domain validation which returns `422 Unprocessable Entity`).
//!
//! The cheapest DoS amplifier on an unauthenticated endpoint is a
//! pathologically large input field (e.g. a 10 MB email string would
//! still trigger SHA-256 hashing + DB query). These checks cut those
//! attack vectors before any expensive work runs.

use crate::error::WebErrorKind;
use crate::Error;

/// RFC 5321 caps email-address length at 254 octets in practice
/// (local-part ≤ 64, domain ≤ 255, total deliverable ≤ 254). Anything
/// longer is not a valid address per the SMTP spec.
pub const MAX_EMAIL_LENGTH: usize = 254;

/// Magic-link tokens are exactly 32 random bytes encoded as URL-safe
/// base64 *without* padding, which always produces a 43-char string.
/// Any other length is impossible for a token we issued and is therefore
/// either a typo, a probe, or an attack.
pub const RAW_TOKEN_LENGTH: usize = 43;

/// Validate an email field at the HTTP boundary.
///
/// Rejects:
/// - Empty strings (cheap garbage)
/// - Lengths > [`MAX_EMAIL_LENGTH`] (DoS amplification)
/// - Strings missing `@` (obvious malformation; not a thorough RFC 5322
///   parse, but enough to reject the cheap stuff)
///
/// Returns `Err(Error::Web(WebErrorKind::Input))` → mapped to `400 Bad Request`.
pub fn validate_email_shape(email: &str) -> Result<(), Error> {
    if email.is_empty() || email.len() > MAX_EMAIL_LENGTH || !email.contains('@') {
        return Err(Error::Web(WebErrorKind::Input));
    }
    Ok(())
}

/// Validate a magic-link token field at the HTTP boundary.
///
/// Rejects anything that isn't exactly [`RAW_TOKEN_LENGTH`] characters.
/// This is a structural cap — any wrong-length input cannot be a token
/// we issued, so we reject it before paying the SHA-256 + DB lookup cost.
///
/// Returns `Err(Error::Web(WebErrorKind::Input))` → mapped to `400 Bad Request`.
pub fn validate_token_length(token: &str) -> Result<(), Error> {
    if token.len() != RAW_TOKEN_LENGTH {
        return Err(Error::Web(WebErrorKind::Input));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_shape_accepts_typical_addresses() {
        for email in [
            "alice@example.com",
            "alice+filter@example.co.uk",
            "a@b.io",
            "very.long.address.with.many.dots@subdomain.example.com",
        ] {
            assert!(validate_email_shape(email).is_ok(), "should accept {email}");
        }
    }

    #[test]
    fn email_shape_rejects_empty() {
        assert!(validate_email_shape("").is_err());
    }

    #[test]
    fn email_shape_rejects_oversized() {
        // 255 chars — one over the RFC 5321 practical cap
        let oversized = "a".repeat(MAX_EMAIL_LENGTH + 1);
        assert!(validate_email_shape(&oversized).is_err());

        // 10 MB email — the headline attack vector
        let attack = "a".repeat(10 * 1024 * 1024);
        assert!(validate_email_shape(&attack).is_err());
    }

    #[test]
    fn email_shape_accepts_at_maximum_length() {
        // Exactly 254 chars, contains '@' — must pass
        let at_limit = format!("a@{}", "b".repeat(MAX_EMAIL_LENGTH - 2));
        assert_eq!(at_limit.len(), MAX_EMAIL_LENGTH);
        assert!(validate_email_shape(&at_limit).is_ok());
    }

    #[test]
    fn email_shape_rejects_missing_at_sign() {
        for email in ["alice", "alice.example.com", "not-an-email-at-all", "junk"] {
            assert!(
                validate_email_shape(email).is_err(),
                "should reject {email}"
            );
        }
    }

    #[test]
    fn token_length_accepts_exact_length() {
        let token = "a".repeat(RAW_TOKEN_LENGTH);
        assert!(validate_token_length(&token).is_ok());
    }

    #[test]
    fn token_length_rejects_wrong_lengths() {
        for length in [0, 1, 42, 44, 64, 1024, 10 * 1024 * 1024] {
            let token = "a".repeat(length);
            assert!(
                validate_token_length(&token).is_err(),
                "should reject length {length}"
            );
        }
    }
}
