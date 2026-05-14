//! User-initiated password reset flow.
//!
//! Reuses the magic-link token infrastructure (with `purpose = PasswordReset`)
//! plus a per-email rate limit and a hard signal-ceiling padding strategy
//! on the response timing. See `docs/architecture/password_reset.md` for
//! the full design and threat model.

use chrono::{Duration as ChronoDuration, Utc};
use entity_api::mutate;
use log::*;
use sea_orm::{DatabaseConnection, IntoActiveModel, TransactionTrait, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::token_purpose::TokenPurpose;
use crate::users;
use entity_api::user::generate_hash;
use service::config::Config;

// Compile-time policy bounds check on HANDLER_TARGET_DURATION_MS.
// Loosening below 100 may not mask DB-jitter variance; raising above
// 500 turns every request into a user-perceptible lag. Build fails
// loudly if someone "tunes" the value without thinking.
const _HANDLER_TARGET_DURATION_BOUNDS_CHECK: () = {
    assert!(
        HANDLER_TARGET_DURATION_MS >= 100,
        "HANDLER_TARGET_DURATION_MS < 100 may not mask DB-jitter variance — \
         changing this requires threat-model review"
    );
    assert!(
        HANDLER_TARGET_DURATION_MS <= 500,
        "HANDLER_TARGET_DURATION_MS > 500 makes every request user-perceptibly slow"
    );
};

/// Target total wall-clock duration for the `request_password_reset` handler.
///
/// Both the unknown-email and known-user paths sleep to reach this target
/// before returning, making the response time deterministic-modulo-DB-jitter.
/// Combined with moving all path-distinguishing work to a background task
/// (see [`process_reset_in_background`]), this gives a **hard signal-ceiling**:
/// timing variance carries no information about which path was taken.
///
/// 150 ms safely overshoots the typical sync-path duration (rate-limit
/// lookup + record-attempt insert, ~15–40 ms in production) while staying
/// well below user-perceptible latency.
const HANDLER_TARGET_DURATION_MS: u64 = 150;

/// Per-email rate-limit window for "no more than one request per N seconds".
const RATE_LIMIT_MIN_INTERVAL_SECS: i64 = 60;

/// Per-email rate-limit cap for "no more than N requests per 24h".
const RATE_LIMIT_DAILY_CAP: u64 = 5;

/// Handle a user-initiated password-reset request.
///
/// Behavior contract:
/// - Returns `Ok(())` whether the email maps to a real user or not
///   (enumeration-safe; the web layer maps this to HTTP 200).
/// - Returns `Err(PasswordResetRateLimited)` when the per-email rate limit
///   is exceeded (web layer maps to HTTP 429). This response is identical
///   whether the email maps to a real user or not.
/// - Returns `Err(...)` for genuine internal failures on the **sync path**
///   only (rate-limit query, attempt-record write). Background work
///   failures are logged but never surface to the caller.
///
/// ## Hard signal-ceiling: how this defeats timing-based enumeration
///
/// All path-distinguishing work — user lookup, token issuance, email send
/// — is moved to a background task ([`process_reset_in_background`]). The
/// only synchronous DB work the response timing depends on is the rate-
/// limit check and attempt-record insert, both of which key on the
/// **email hash** and never read the user record. They take identical
/// time regardless of whether the email maps to a real user.
///
/// After the sync work, the handler pads to [`HANDLER_TARGET_DURATION_MS`]
/// (150 ms) before returning. The result: response timing is
/// deterministic-modulo-DB-jitter, and whatever jitter exists is
/// identical across both branches — so it carries no enumeration signal.
///
/// This is stronger than padding the slow path alone: even a perfect pad
/// on the known-user branch cannot fully mask the MailerSend HTTPS round-
/// trip (100–500 ms with high variance) on the known path, which would
/// otherwise dominate response timing. Moving the email send (and the
/// other path-distinguishing operations) off the response path eliminates
/// the bimodal distribution at its source.
///
/// ## API semantic change vs a purely synchronous design
///
/// "200 OK" means "we accepted your reset request" — not "we processed
/// it." Token issuance and email send happen after the response returns.
/// For password reset this is acceptable because email send was already
/// best-effort (a synchronous design also returns 200 even if the email
/// fails to enqueue) and the wire contract is committed to
/// enumeration-safe always-200 anyway.
pub async fn request_password_reset(
    db: Arc<DatabaseConnection>,
    email: &str,
    config: &Config,
) -> Result<(), Error> {
    let handler_start = Instant::now();

    // === SYNCHRONOUS CRITICAL PATH ===
    // These operations must complete before the response. They all key
    // on `email_hash`, NEVER on the user record, so their timing is
    // identical regardless of whether the email maps to a real user.

    let email_hash = hash_email(email);
    enforce_rate_limit(&db, &email_hash).await?;
    entity_api::password_reset_attempt::record(&*db, &email_hash).await?;

    // === BACKGROUND CONTINUATION ===
    // Spawn everything path-distinguishing (user lookup, token issuance,
    // email send) into a tokio task. Response timing no longer depends
    // on whether the email matches a user.
    //
    // Cloning the `Arc<DatabaseConnection>` is cheap — it bumps the
    // refcount on the shared connection pool, not the pool itself.
    let db_for_bg = Arc::clone(&db);
    let email_for_bg = email.to_string();
    let config_for_bg = config.clone();
    tokio::spawn(async move {
        process_reset_in_background(db_for_bg, email_for_bg, config_for_bg).await;
    });

    // === PAD TO TARGET ===
    // Both paths return at approximately HANDLER_TARGET_DURATION_MS.
    // DB-jitter variance affects both paths equally (the sync ops above
    // are path-identical), so timing carries no enumeration signal.
    pad_handler_duration(handler_start).await;
    Ok(())
}

/// Path-distinguishing work for a password-reset request, run as a
/// background `tokio::spawn` after the handler has already responded.
///
/// All errors are logged at WARN and discarded — by the time this runs,
/// the client has already received `200 OK`. Failure modes:
/// - **Unknown email**: logged at WARN; no further action.
/// - **DB error during user lookup or token issuance**: logged at WARN;
///   no further action. The user can request a fresh reset.
/// - **Email-send failure**: logged at WARN; the token still exists and
///   will expire naturally. The user can request a fresh reset.
async fn process_reset_in_background(db: Arc<DatabaseConnection>, email: String, config: Config) {
    let user = match entity_api::user::find_by_email(&*db, &email).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            warn!("[password-reset] reset requested for unknown email (no user match)");
            debug!("[password-reset] unknown-email value was: {email}");
            return;
        }
        Err(e) => {
            warn!("[password-reset] background user lookup failed: {e:?}");
            return;
        }
    };

    let expiry_seconds = config.password_reset_token_expiry_seconds() as i64;
    let raw_token = match crate::magic_link_token::create_magic_link(
        &db,
        user.id,
        expiry_seconds,
        TokenPurpose::PasswordReset,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            warn!(
                "[password-reset] background token creation failed for user {}: {e:?}",
                user.id
            );
            return;
        }
    };

    if let Err(e) = crate::emails::send_password_reset_email(&config, &user, &raw_token).await {
        warn!(
            "[password-reset] failed to send email to user {}: {e:?}",
            user.id
        );
    }

    warn!("[password-reset] reset link issued for user {}", user.id);
}

/// Sleep enough to ensure the handler took at least
/// `HANDLER_TARGET_DURATION_MS` wall-clock time. No-op if the sync work
/// already exceeded the target (which happens only under pathological
/// DB latency; the overshoot affects both paths equally so it carries
/// no enumeration signal).
async fn pad_handler_duration(start: Instant) {
    let target = Duration::from_millis(HANDLER_TARGET_DURATION_MS);
    let elapsed = start.elapsed();
    if elapsed < target {
        sleep(target - elapsed).await;
    }
}

/// Normalize an email (lowercase + trim) and return its SHA-256 hex digest.
///
/// Normalization ensures `Foo@Example.COM` and ` foo@example.com ` share
/// a rate-limit bucket — without it, capitalization variants could bypass
/// the limit. Hash-then-key prevents the audit table from storing email
/// plaintext (modest defense-in-depth against DB leak).
fn hash_email(email: &str) -> String {
    let normalized = email.trim().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex::encode(hasher.finalize())
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

    // Server-side policy enforcement — independent of any FE validation.
    // See `domain::password_policy` for the rules.
    crate::password_policy::validate_password(&password)?;

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
/// Both checks query the **`password_reset_attempts`** append-only audit
/// table keyed by `email_hash`, NOT `user_id`. Keying by email hash means
/// the rate limit applies before — and uniformly to — both the unknown-
/// email and known-user paths, closing the 200/429 enumeration oracle.
///
/// Returns `PasswordResetRateLimited` on either breach (mapped to HTTP 429
/// by the web layer). The same `[password-reset] rate-limited` WARN line
/// fires regardless of whether the email matched a user.
async fn enforce_rate_limit(db: &DatabaseConnection, email_hash: &str) -> Result<(), Error> {
    let most_recent = entity_api::password_reset_attempt::find_most_recent(db, email_hash).await?;

    if let Some(attempt) = most_recent {
        let elapsed = Utc::now() - attempt.attempted_at.with_timezone(&Utc);
        if elapsed < ChronoDuration::seconds(RATE_LIMIT_MIN_INTERVAL_SECS) {
            warn!("[password-reset] rate-limited (min-interval)");
            return Err(rate_limited_error());
        }
    }

    let since = (Utc::now() - ChronoDuration::hours(24)).into();
    let recent_count =
        entity_api::password_reset_attempt::count_since(db, email_hash, since).await?;

    if recent_count >= RATE_LIMIT_DAILY_CAP {
        warn!("[password-reset] rate-limited (daily-cap)");
        return Err(rate_limited_error());
    }

    Ok(())
}

/// Sweep old password-reset attempt records.
///
/// **Intended call pattern**: a nightly ops job (or ad-hoc invocation) running
/// against the production DB. Safe to call concurrently with live request
/// traffic — under PostgreSQL MVCC, an INSERT with `attempted_at = NOW()`
/// is outside the `< cutoff` predicate and is not affected.
///
/// # Retention policy
///
/// The daily-cap check looks back 24 hours, so anything older than that is
/// not strictly required for rate-limit correctness. We keep records for a
/// longer window for **security forensics** — if a user reports "someone
/// kept trying to reset my password last week" we want the audit trail to
/// survive long enough to investigate. Recommended retention: **30 days**.
/// Anything shorter than ~2 days defeats forensic value; anything longer
/// than ~90 days grows the table needlessly.
///
/// # Returns
///
/// Number of rows deleted. Logged at INFO when non-zero so ops can confirm
/// the job ran.
///
/// # Ad-hoc invocation (psql)
///
/// ```sql
/// -- Dry-run: how many rows would be removed?
/// SELECT COUNT(*) FROM refactor_platform.password_reset_attempts
/// WHERE attempted_at < NOW() - INTERVAL '30 days';
///
/// -- Actually delete:
/// DELETE FROM refactor_platform.password_reset_attempts
/// WHERE attempted_at < NOW() - INTERVAL '30 days';
/// ```
///
/// # Errors
///
/// Returns `Err(Validation)` if `retention_days < 1` (would purge records
/// still needed for the 24-hour daily-cap window).
pub async fn sweep_old_attempts(
    db: &DatabaseConnection,
    retention_days: i64,
) -> Result<u64, Error> {
    if retention_days < 1 {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation(format!(
                "retention_days must be >= 1 (got {retention_days}); shorter retention would \
                 purge records needed for the 24-hour daily-cap rate-limit check"
            )),
        });
    }

    let cutoff = (Utc::now() - ChronoDuration::days(retention_days)).into();
    let deleted = entity_api::password_reset_attempt::delete_older_than(db, cutoff).await?;

    if deleted > 0 {
        info!(
            "[password-reset] sweep removed {} attempt record(s) older than {} day(s)",
            deleted, retention_days
        );
    } else {
        debug!(
            "[password-reset] sweep removed 0 records older than {} day(s)",
            retention_days
        );
    }

    Ok(deleted)
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
    ///
    /// Post-fix flow on the unknown-email path:
    ///   find_most_recent (None) → count_since (0) → record (insert) → find_by_email (None) → padding → Ok
    #[tokio::test]
    async fn request_password_reset_returns_ok_when_user_not_found() {
        use sea_orm::MockExecResult;

        let attempt_row = password_reset_attempts::Model {
            id: crate::Id::new_v4(),
            email_hash: hash_email("nobody@example.com"),
            attempted_at: Utc::now().into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // 1. find_most_recent → None (no prior attempts for this email)
            .append_query_results(vec![Vec::<password_reset_attempts::Model>::new()])
            // 2. count_since (.all().len()) → 0
            .append_query_results(vec![Vec::<password_reset_attempts::Model>::new()])
            // 3. record (INSERT) — returns the inserted row
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![attempt_row]])
            // 4. find_by_email (find_with_related) → no user
            .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>(vec![vec![]])
            .into_connection();

        let config = Config::default();
        let result = request_password_reset(Arc::new(db), "nobody@example.com", &config).await;

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

    // ----- Rate-limit tests (regression coverage for the PR #311 review fix) -----
    //
    // Before the fix, `enforce_rate_limit` counted rows in `magic_link_tokens`,
    // which is a state table (one live row per user/purpose). The count was
    // therefore always 0 or 1, and the 5/24h daily-cap check was unreachable.
    // The tests below exercise the new audit-table path against MockDatabase.

    use crate::password_reset_attempts;
    use crate::user_roles;

    fn test_user(email: &str) -> users::Model {
        users::Model {
            id: crate::Id::new_v4(),
            email: email.into(),
            first_name: "Test".into(),
            last_name: "User".into(),
            display_name: None,
            password: Some("already-set".into()),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".into(),
            role: Default::default(),
            roles: vec![],
            invite_status: None,
            created_at: Utc::now().into(),
            updated_at: Utc::now().into(),
        }
    }

    fn test_attempt(email_hash: &str, age: ChronoDuration) -> password_reset_attempts::Model {
        password_reset_attempts::Model {
            id: crate::Id::new_v4(),
            email_hash: email_hash.to_string(),
            attempted_at: (Utc::now() - age).into(),
        }
    }

    /// Min-interval guard for a KNOWN-user email: a recent attempt within
    /// `RATE_LIMIT_MIN_INTERVAL_SECS` must trip `PasswordResetRateLimited`.
    ///
    /// Post-fix flow: find_most_recent fires BEFORE find_by_email, so the
    /// test does not need to mock the user lookup at all — the rate-limit
    /// short-circuits the function before it gets there.
    #[tokio::test]
    async fn request_password_reset_returns_429_on_min_interval() {
        let email = "alice@example.com";
        let email_hash = hash_email(email);
        let recent_attempt = test_attempt(&email_hash, ChronoDuration::seconds(10));

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // find_most_recent → 10s-old attempt; short-circuits before user lookup
            .append_query_results(vec![vec![recent_attempt]])
            .into_connection();

        let err = request_password_reset(Arc::new(db), email, &Config::default())
            .await
            .expect_err("expected PasswordResetRateLimited");

        assert_eq!(
            err.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::PasswordResetRateLimited
            )),
            "recent attempt within min-interval window must trip the rate limit"
        );
    }

    /// **Enumeration-oracle regression test.**
    ///
    /// Before this fix, the unknown-email path skipped `enforce_rate_limit`,
    /// so a second request from an attacker would return 200 (unknown email)
    /// vs 429 (known user) — a deterministic existence oracle on the second
    /// request. Post-fix, the rate limit fires uniformly on both paths.
    ///
    /// This test mocks the unknown-email path AND a recent prior attempt
    /// for the same email hash: the function must return 429 even though
    /// the email has no matching user.
    #[tokio::test]
    async fn request_password_reset_rate_limits_unknown_email_path_too() {
        let email = "nobody@example.com";
        let email_hash = hash_email(email);
        let recent_attempt = test_attempt(&email_hash, ChronoDuration::seconds(10));

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // find_most_recent → 10s-old attempt (recorded by attacker's first probe)
            .append_query_results(vec![vec![recent_attempt]])
            // (find_by_email never runs — short-circuited by rate limit)
            .into_connection();

        let err = request_password_reset(Arc::new(db), email, &Config::default())
            .await
            .expect_err("expected PasswordResetRateLimited on unknown-email path");

        assert_eq!(
            err.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::PasswordResetRateLimited
            )),
            "unknown-email path MUST rate-limit too — asymmetric handling is an enumeration oracle"
        );
    }

    /// Daily-cap guard: 5 attempts in the last 24h must trip the rate limit.
    /// This is the bug the PR #311 review caught — `magic_link_tokens` could
    /// only ever return count=1 because delete-then-create wipes prior rows.
    /// The new audit table keeps history and the cap is actually reachable.
    #[tokio::test]
    async fn request_password_reset_returns_429_on_daily_cap() {
        let email = "alice@example.com";
        let email_hash = hash_email(email);
        // Most-recent attempt was 5 minutes ago — past the min-interval
        // window so the daily-cap check is reached, but well within 24h.
        let most_recent = test_attempt(&email_hash, ChronoDuration::minutes(5));
        // Five attempts across the last 24h (cap is 5, check is `>= cap`).
        let attempts_24h: Vec<password_reset_attempts::Model> = (1..=5)
            .map(|h| test_attempt(&email_hash, ChronoDuration::hours(h)))
            .collect();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // 1. find_most_recent → 5min ago (past min-interval)
            .append_query_results(vec![vec![most_recent]])
            // 2. count_since (.all().len()) → 5 rows in window
            .append_query_results(vec![attempts_24h])
            .into_connection();

        let err = request_password_reset(Arc::new(db), email, &Config::default())
            .await
            .expect_err("expected PasswordResetRateLimited at daily cap");

        assert_eq!(
            err.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::PasswordResetRateLimited
            )),
            "5 attempts in last 24h must trip the daily-cap (was unreachable before the fix)"
        );
    }

    /// Email normalization must collapse capitalization and surrounding
    /// whitespace into a single rate-limit bucket. Without normalization,
    /// an attacker could probe `Alice@example.com`, ` alice@example.com `,
    /// and `ALICE@example.com` as three independent rate-limit keys and
    /// trivially bypass the 5/24h cap.
    #[test]
    fn hash_email_normalizes_case_and_whitespace() {
        let canonical = hash_email("alice@example.com");
        assert_eq!(hash_email("ALICE@example.com"), canonical);
        assert_eq!(hash_email("Alice@Example.COM"), canonical);
        assert_eq!(hash_email("  alice@example.com  "), canonical);
        assert_eq!(hash_email("alice@example.com\n"), canonical);
        // Different addresses must NOT collide.
        assert_ne!(hash_email("bob@example.com"), canonical);
        // SHA-256 hex digest is 64 chars.
        assert_eq!(canonical.len(), 64);
    }

    /// Test-fixture sanity — keeps the `test_user` helper referenced so
    /// future tests that need a known user can construct one.
    #[test]
    fn test_user_helper_produces_distinct_uuids() {
        let a = test_user("a@example.com");
        let b = test_user("b@example.com");
        assert_ne!(a.id, b.id);
    }

    /// Defense-in-depth on the ops sweep API: retention shorter than 1 day
    /// would purge records still needed for the 24-hour daily-cap window.
    /// Reject with a `Validation` error so misuse fails loudly rather than
    /// silently corrupting the rate-limit state.
    #[tokio::test]
    async fn sweep_old_attempts_rejects_zero_or_negative_retention() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        for bad in [0_i64, -1, -30] {
            let err = sweep_old_attempts(&db, bad)
                .await
                .expect_err("expected Validation error for retention_days < 1");

            match err.error_kind {
                DomainErrorKind::Validation(_) => {} // expected
                other => panic!("expected DomainErrorKind::Validation for {bad}, got {other:?}"),
            }
        }
    }

    /// Confirm the sweep delegates to the entity_api correctly and returns
    /// the count from the underlying delete. Uses MockDatabase's exec result
    /// to simulate the DELETE's `rows_affected`.
    #[tokio::test]
    async fn sweep_old_attempts_returns_deleted_count() {
        use sea_orm::MockExecResult;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 42,
            }])
            .into_connection();

        let count = sweep_old_attempts(&db, 30)
            .await
            .expect("sweep with valid retention must succeed");

        assert_eq!(count, 42, "sweep must return rows_affected from the DELETE");
    }

    /// Hard signal-ceiling: `request_password_reset` must always take at
    /// least `HANDLER_TARGET_DURATION_MS` wall-clock time. Combined with
    /// the spawn-everything-path-distinguishing structural change, this
    /// guarantees response time carries no information about which branch
    /// the request followed.
    ///
    /// This test proves the *floor* component of the signal-ceiling. The
    /// *ceiling* (that variance doesn't differ between branches) is a
    /// structural property of the code — the sync path no longer touches
    /// the user record — and is verified by reading the implementation,
    /// not by a single-run timing assertion.
    #[tokio::test]
    async fn request_password_reset_pads_response_to_target_duration() {
        use sea_orm::MockExecResult;

        let attempt_row = password_reset_attempts::Model {
            id: crate::Id::new_v4(),
            email_hash: hash_email("anyone@example.com"),
            attempted_at: Utc::now().into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // Sync path: rate-limit (None) → count_since (0) → record (insert)
            .append_query_results(vec![Vec::<password_reset_attempts::Model>::new()])
            .append_query_results(vec![Vec::<password_reset_attempts::Model>::new()])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![attempt_row]])
            .into_connection();

        let start = std::time::Instant::now();
        let result =
            request_password_reset(Arc::new(db), "anyone@example.com", &Config::default()).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "expected Ok response; got {result:?}");

        // Allow a small margin for clock resolution and tokio sleep precision
        // (typically <1ms on idle systems, more under heavy CI load).
        let min_expected = Duration::from_millis(HANDLER_TARGET_DURATION_MS.saturating_sub(10));
        assert!(
            elapsed >= min_expected,
            "handler must pad to at least HANDLER_TARGET_DURATION_MS ({HANDLER_TARGET_DURATION_MS}ms); \
             took {elapsed:?}, expected ≥ {min_expected:?}. \
             A failure here means the signal-ceiling defense is degraded."
        );
    }

    /// The target padding constant must stay in the "imperceptible-to-user
    /// but generous-over-DB-jitter" range. Loosening below 100ms risks not
    /// masking real DB-jitter variance; raising above ~500ms turns every
    // HANDLER_TARGET_DURATION_MS bounds are guarded at compile time —
    // see the const assertion at module scope (`_HANDLER_TARGET_DURATION_BOUNDS_CHECK`).
    // Build fails if someone loosens the value.

    /// Server-side password policy must fire from `complete_password_reset`,
    /// independently of any FE validation. Empty password → 422 Validation,
    /// even if the FE bug or malicious client lets one through.
    #[tokio::test]
    async fn complete_password_reset_rejects_empty_password() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut params = entity_api::mutate::UpdateMap::new();
        params.insert(
            "password".into(),
            Some(Value::String(Some(Box::new("".into())))),
        );
        params.insert(
            "confirm_password".into(),
            Some(Value::String(Some(Box::new("".into())))),
        );
        params.insert(
            "token".into(),
            Some(Value::String(Some(Box::new("any_token".into())))),
        );

        struct P(entity_api::mutate::UpdateMap);
        impl entity_api::mutate::IntoUpdateMap for P {
            fn into_update_map(self) -> entity_api::mutate::UpdateMap {
                self.0
            }
        }

        let err = complete_password_reset(&db, P(params))
            .await
            .expect_err("empty password must be rejected before token validation");

        match err.error_kind {
            DomainErrorKind::Validation(msg) => {
                assert!(
                    msg.contains("empty") || msg.contains("whitespace"),
                    "expected empty/whitespace message, got: {msg}"
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// And the policy must reject too-short passwords too, not just empty.
    #[tokio::test]
    async fn complete_password_reset_rejects_too_short_password() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut params = entity_api::mutate::UpdateMap::new();
        // 8 chars — passes confirm-match but fails min-length policy.
        params.insert(
            "password".into(),
            Some(Value::String(Some(Box::new("short123".into())))),
        );
        params.insert(
            "confirm_password".into(),
            Some(Value::String(Some(Box::new("short123".into())))),
        );
        params.insert(
            "token".into(),
            Some(Value::String(Some(Box::new("any_token".into())))),
        );

        struct P(entity_api::mutate::UpdateMap);
        impl entity_api::mutate::IntoUpdateMap for P {
            fn into_update_map(self) -> entity_api::mutate::UpdateMap {
                self.0
            }
        }

        let err = complete_password_reset(&db, P(params))
            .await
            .expect_err("8-char password must be rejected by min-length policy");

        match err.error_kind {
            DomainErrorKind::Validation(msg) => {
                assert!(
                    msg.contains("12"),
                    "error message must name the minimum length: {msg}"
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }
}
