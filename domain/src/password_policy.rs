//! Server-side password policy enforcement.
//!
//! The backend enforces these rules regardless of what the frontend sends —
//! defense in depth against client-side validation bugs, malicious JS,
//! or direct API calls that bypass the form. The frontend should mirror
//! these rules for instant user feedback, but the BE is the security
//! boundary.
//!
//! See `docs/architecture/password_reset.md` for the policy decision and
//! the rationale (NIST 800-63B recommendations, no complexity rules).
//!
//! Applied at:
//! - [`crate::password_reset::complete_password_reset`]
//! - [`crate::magic_link_token::complete_setup`]

use crate::error::{DomainErrorKind, Error};

/// Minimum password length in **characters** (Unicode scalar values).
///
/// 12 is the modern industry baseline. NIST 800-63B requires ≥8; 12 raises
/// the bar against offline brute-force while remaining typeable by humans.
pub const MIN_PASSWORD_LENGTH: usize = 12;

/// Maximum password length in characters.
///
/// argon2 will happily hash arbitrarily long inputs, but the hashing cost
/// grows with input size. Capping at 128 prevents a DoS where an attacker
/// submits multi-megabyte "passwords" to exhaust CPU. 128 chars is well
/// above any reasonable user password or password-manager output.
pub const MAX_PASSWORD_LENGTH: usize = 128;

/// Validate a candidate password against the BE-enforced policy.
///
/// Returns `Err(DomainErrorKind::Validation)` with a user-facing message
/// when the password is rejected. Currently checks: non-empty after trim,
/// length in `[MIN_PASSWORD_LENGTH, MAX_PASSWORD_LENGTH]`.
///
/// Deliberately does NOT enforce character-class complexity rules
/// (uppercase / lowercase / digit / symbol). NIST 800-63B recommends
/// *against* such rules — they push users toward predictable patterns
/// (e.g. `Password1!`) that reduce real-world entropy. Length is the
/// load-bearing dimension; raise the minimum if more entropy is needed.
pub fn validate_password(password: &str) -> Result<(), Error> {
    if password.trim().is_empty() {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation(
                "Password cannot be empty or whitespace".to_string(),
            ),
        });
    }

    let length = password.chars().count();
    if length < MIN_PASSWORD_LENGTH {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation(format!(
                "Password must be at least {MIN_PASSWORD_LENGTH} characters"
            )),
        });
    }
    if length > MAX_PASSWORD_LENGTH {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation(format!(
                "Password must be at most {MAX_PASSWORD_LENGTH} characters"
            )),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_password() {
        let err = validate_password("").expect_err("empty password must be rejected");
        match err.error_kind {
            DomainErrorKind::Validation(msg) => {
                assert!(msg.contains("empty") || msg.contains("whitespace"));
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn rejects_whitespace_only_password() {
        // Various whitespace combinations
        for password in ["   ", "\t", "\n", " \t \n "] {
            let err =
                validate_password(password).expect_err("whitespace-only password must be rejected");
            assert!(
                matches!(err.error_kind, DomainErrorKind::Validation(_)),
                "expected Validation for {password:?}"
            );
        }
    }

    #[test]
    fn rejects_too_short_password() {
        // 11 chars — one below the minimum
        let err = validate_password("12345678901").expect_err("11-char password must be rejected");
        match err.error_kind {
            DomainErrorKind::Validation(msg) => {
                assert!(
                    msg.contains("12"),
                    "error message must name the policy: {msg}"
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn accepts_password_at_minimum_length() {
        // exactly 12 chars
        validate_password("123456789012").expect("12-char password must be accepted");
    }

    #[test]
    fn accepts_password_at_maximum_length() {
        let password: String = "x".repeat(MAX_PASSWORD_LENGTH);
        validate_password(&password).expect("password at MAX_PASSWORD_LENGTH must be accepted");
    }

    #[test]
    fn rejects_too_long_password() {
        let password: String = "x".repeat(MAX_PASSWORD_LENGTH + 1);
        let err = validate_password(&password).expect_err("over-max password must be rejected");
        match err.error_kind {
            DomainErrorKind::Validation(msg) => {
                assert!(
                    msg.contains("128"),
                    "error message must name the policy: {msg}"
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn unicode_password_counts_by_char_not_byte() {
        // 12 emoji = 12 chars, but ~48 bytes (each emoji is 4 bytes in UTF-8).
        // The check must use char-count, so this must be ACCEPTED (12 chars >= 12 min).
        let twelve_emoji: String = "🔐".repeat(12);
        validate_password(&twelve_emoji).expect("12 unicode scalar values must be accepted");

        // 11 emoji = 11 chars — must be REJECTED (below min).
        let eleven_emoji: String = "🔐".repeat(11);
        validate_password(&eleven_emoji).expect_err("11 unicode scalar values must be rejected");
    }
}
