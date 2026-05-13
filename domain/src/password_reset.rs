//! User-initiated password reset flow.
//!
//! Reuses the magic-link token infrastructure (with `purpose = PasswordReset`)
//! plus a per-email rate limit and a constant-time padding step on the
//! email-not-found path. See `docs/architecture/password_reset.md` for the
//! full design and threat model.

use chrono::{Duration as ChronoDuration, Utc};
use entity_api::mutate;
use log::*;
use sea_orm::{DatabaseConnection, IntoActiveModel, TransactionTrait, Value};
use std::time::Duration;
use tokio::time::sleep;

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::token_purpose::TokenPurpose;
use crate::users;
use entity_api::user::generate_hash;
use service::config::Config;

/// Padding applied to the email-not-found code path to defeat timing-based
/// user enumeration. Defends against an attacker measuring response latency
/// to distinguish "user exists" (slower — DB writes + email enqueue) from
/// "user doesn't exist" (faster — single DB read).
///
/// 75 ms is enough to mask the typical 5–20 ms difference between paths
/// while staying well below user-perceivable response time.
const ENUMERATION_PADDING_MS: u64 = 75;

/// Per-email rate-limit window for "no more than one request per N seconds".
const RATE_LIMIT_MIN_INTERVAL_SECS: i64 = 60;

/// Per-email rate-limit cap for "no more than N requests per 24h".
const RATE_LIMIT_DAILY_CAP: u64 = 5;

/// Handle a user-initiated password-reset request.
///
/// Behavior contract:
/// - Returns `Ok(())` whether the email maps to a real user or not
///   (enumeration-safe; the web layer maps this to HTTP 200).
/// - Returns `Err(InvalidOrExpiredToken)` is **not** possible from this
///   endpoint — that error only occurs at `validate` / `complete` time.
/// - Returns `Err(PasswordResetRateLimited)` when the per-email rate limit
///   is exceeded (web layer maps to HTTP 429).
/// - Returns `Err(...)` for genuine internal failures (DB, config) — the
///   web layer maps those to 5xx.
///
/// Email sending is best-effort: a downstream MailerSend failure is logged
/// but does not propagate to the caller (preserves the 200 contract). Token
/// creation, however, is required — if the DB transaction fails we surface
/// it.
pub async fn request_password_reset(
    db: &DatabaseConnection,
    email: &str,
    config: &Config,
) -> Result<(), Error> {
    let user = entity_api::user::find_by_email(db, email).await?;

    let Some(user) = user else {
        // Constant-time padding: do NOT distinguish "no such user" from
        // the success path via response latency. The WARN is a security
        // signal ("someone tried to reset an unknown account") but the
        // raw email is PII and stays at DEBUG.
        warn!("[password-reset] reset requested for unknown email (no user match)");
        debug!("[password-reset] unknown-email value was: {email}");
        sleep(Duration::from_millis(ENUMERATION_PADDING_MS)).await;
        return Ok(());
    };

    enforce_rate_limit(db, user.id).await?;

    let expiry_seconds = config.password_reset_token_expiry_seconds() as i64;
    let raw_token = crate::magic_link_token::create_magic_link(
        db,
        user.id,
        expiry_seconds,
        TokenPurpose::PasswordReset,
    )
    .await?;

    // Email delivery is best-effort. Failure is logged but does not change
    // the response contract (still 200 to the FE).
    if let Err(e) = crate::emails::send_password_reset_email(config, &user, &raw_token).await {
        warn!(
            "[password-reset] failed to send email to user {}: {e:?}",
            user.id
        );
    }

    warn!("[password-reset] reset link issued for user {}", user.id);
    Ok(())
}

/// Validate a raw password-reset token without consuming it.
///
/// Returns the associated user on success. Maps any underlying validation
/// failure (not found, expired, wrong purpose) to the collapsed
/// `InvalidOrExpiredToken` error so callers can't distinguish them.
pub async fn validate_reset_token(
    db: &DatabaseConnection,
    raw_token: &str,
) -> Result<users::Model, Error> {
    crate::magic_link_token::validate_token(db, raw_token, TokenPurpose::PasswordReset)
        .await
        .map_err(collapse_to_invalid_or_expired)
}

/// Consume a password-reset token and set the user's new password.
///
/// Atomic: validation, token deletion, and password update happen in one
/// transaction. On failure, no state changes.
pub async fn complete_password_reset(
    db: &DatabaseConnection,
    params: impl mutate::IntoUpdateMap,
) -> Result<users::Model, Error> {
    let mut params = params.into_update_map();

    let password = params.remove("password")?;
    let confirm_password = params.remove("confirm_password")?;
    let raw_token = params.remove("token")?;

    if password != confirm_password {
        warn!("[password-reset] password confirmation mismatch on /complete");
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation(
                "Password confirmation does not match".to_string(),
            ),
        });
    }

    params.insert(
        "password".to_string(),
        Some(Value::String(Some(Box::new(generate_hash(password))))),
    );

    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    let user =
        crate::magic_link_token::validate_token(&txn, &raw_token, TokenPurpose::PasswordReset)
            .await
            .map_err(collapse_to_invalid_or_expired)?;

    entity_api::magic_link_token::delete_all_for_user(&txn, user.id, TokenPurpose::PasswordReset)
        .await?;

    let active_model = user.into_active_model();
    let updated_user =
        mutate::update::<users::ActiveModel, users::Column>(&txn, active_model, params).await?;

    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    warn!(
        "[password-reset] user {} completed password reset (password changed)",
        updated_user.id
    );
    Ok(updated_user)
}

/// Check the per-email rate limit for password-reset requests.
///
/// Two checks ANDed:
/// 1. No new request within `RATE_LIMIT_MIN_INTERVAL_SECS` of the previous
///    request (catches rapid-fire abuse).
/// 2. No more than `RATE_LIMIT_DAILY_CAP` requests in the last 24 hours
///    (catches slower but persistent abuse).
///
/// Both checks query the `magic_link_tokens` table scoped to
/// `purpose = PasswordReset`. Returns `PasswordResetRateLimited` on either
/// breach.
async fn enforce_rate_limit(db: &DatabaseConnection, user_id: crate::Id) -> Result<(), Error> {
    let most_recent = entity_api::magic_link_token::find_most_recent_for_user(
        db,
        user_id,
        TokenPurpose::PasswordReset,
    )
    .await?;

    if let Some(token) = most_recent {
        let elapsed = Utc::now() - token.created_at.with_timezone(&Utc);
        if elapsed < ChronoDuration::seconds(RATE_LIMIT_MIN_INTERVAL_SECS) {
            warn!(
                "[password-reset] rate-limited (min-interval) for user {}",
                user_id
            );
            return Err(rate_limited_error());
        }
    }

    let since = (Utc::now() - ChronoDuration::hours(24)).into();
    let recent_count = entity_api::magic_link_token::count_for_user_since(
        db,
        user_id,
        TokenPurpose::PasswordReset,
        since,
    )
    .await?;

    if recent_count >= RATE_LIMIT_DAILY_CAP {
        warn!(
            "[password-reset] rate-limited (daily-cap) for user {}",
            user_id
        );
        return Err(rate_limited_error());
    }

    Ok(())
}

/// Collapse any token-validation error (NotFound, Unauthenticated, etc.)
/// to the uniform `InvalidOrExpiredToken` so callers can't distinguish
/// "token never existed" from "token expired" from "wrong purpose."
fn collapse_to_invalid_or_expired(err: Error) -> Error {
    // Preserve only the underlying source for log tracing; the error_kind
    // is replaced with the collapsed discriminator.
    Error {
        source: err.source,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::InvalidOrExpiredToken,
        )),
    }
}

fn rate_limited_error() -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::PasswordResetRateLimited,
        )),
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use crate::magic_link_tokens;
    use crate::users;
    use chrono::Duration as ChronoDuration;
    use sea_orm::{DatabaseBackend, MockDatabase};

    /// Enumeration safety: when the email maps to no user, `request_password_reset`
    /// must return `Ok(())` (the web layer maps this to 200). The controller cannot
    /// distinguish "email exists" from "email does not exist" via the response.
    #[tokio::test]
    async fn request_password_reset_returns_ok_when_user_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // find_by_email returns no rows
            .append_query_results(vec![Vec::<users::Model>::new()])
            .into_connection();

        let config = Config::default();
        let result = request_password_reset(&db, "nobody@example.com", &config).await;

        assert!(
            result.is_ok(),
            "Expected Ok(()) for missing email; got {result:?}"
        );
    }

    /// Token-state opacity: any underlying validation failure (token not found,
    /// expired, wrong purpose) must be reported to callers as the collapsed
    /// `InvalidOrExpiredToken`. The web layer maps this to a uniform
    /// `400 invalid_or_expired_token` — no timing or status oracle for attackers
    /// spraying random tokens.
    #[tokio::test]
    async fn validate_reset_token_collapses_not_found_to_invalid_or_expired() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // find_by_token_hash returns None
            .append_query_results(vec![Vec::<magic_link_tokens::Model>::new()])
            .into_connection();

        let err = validate_reset_token(&db, "any_raw_token")
            .await
            .expect_err("expected InvalidOrExpiredToken");

        assert_eq!(
            err.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::InvalidOrExpiredToken
            )),
            "underlying NotFound must be collapsed to InvalidOrExpiredToken"
        );
    }

    /// The same collapse applies when the token exists but is expired.
    /// (Underlying error would be `Unauthenticated`; surface must be uniform.)
    #[tokio::test]
    async fn validate_reset_token_collapses_expired_to_invalid_or_expired() {
        let expired_token = magic_link_tokens::Model {
            id: crate::Id::new_v4(),
            user_id: crate::Id::new_v4(),
            token_hash: "irrelevant".into(),
            expires_at: (Utc::now() - ChronoDuration::hours(1)).into(),
            created_at: Utc::now().into(),
            purpose: TokenPurpose::PasswordReset,
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expired_token]])
            .into_connection();

        let err = validate_reset_token(&db, "expired_token")
            .await
            .expect_err("expected InvalidOrExpiredToken");

        assert_eq!(
            err.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::InvalidOrExpiredToken
            )),
            "expired token must be collapsed to InvalidOrExpiredToken (not Unauthenticated)"
        );
    }
}
