use super::error::Error;

use chrono::Utc;
use entity::password_reset_attempts::{ActiveModel, Column, Entity, Model};
use sea_orm::{entity::prelude::*, ConnectionTrait, DbBackend, QueryOrder, Set, Statement};

/// Acquire a PostgreSQL transaction-scoped advisory lock keyed on the
/// SHA-256 email hash. Concurrent transactions touching the same email
/// serialize on this lock; different emails proceed in parallel.
///
/// **Must be called inside a transaction.** The lock is released
/// automatically when the enclosing transaction commits or rolls back.
///
/// Used to close the TOCTOU race between the rate-limit check and the
/// attempt-record write: without the lock, two concurrent requests with
/// the same `email_hash` could both pass the rate-limit check (each
/// reads a snapshot showing no recent attempts) before either has
/// written its attempt row, then both insert and both fire emails.
///
/// PostgreSQL-specific. Uses `pg_advisory_xact_lock(bigint)`; the
/// `hashtext($1)::bigint` cast hashes our 64-char hex email-hash down
/// to the int64 the lock function expects. Collision probability on
/// 64 bits across two unrelated emails is negligible, and a collision
/// would only cause unnecessary serialization, not a correctness bug.
///
/// `tower_governor` / SeaORM don't abstract advisory locks — this is
/// a raw `SELECT` passed through SeaORM's parameterized `Statement`
/// API, matching the project's pattern for other PG-specific calls
/// (see migrations).
pub async fn lock_email_hash(txn: &impl ConnectionTrait, email_hash: &str) -> Result<(), Error> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
        [email_hash.into()],
    );
    txn.execute(stmt).await?;
    Ok(())
}

/// Append an attempt row keyed by the SHA-256 hex digest of the normalized
/// email. This table is the source of truth for rate-limiting; it is
/// intentionally append-only (no UPDATE / DELETE on the request path) so
/// the row count over a time window is meaningful.
///
/// Recording is keyed by email-hash, NOT by `user_id`, so that unknown-
/// email and known-user requests are treated uniformly. Asymmetric rate-
/// limiting between the two paths would be an enumeration oracle on its
/// own — see `docs/architecture/password_reset.md` for the design.
pub async fn record(db: &impl ConnectionTrait, email_hash: &str) -> Result<Model, Error> {
    let active_model = ActiveModel {
        email_hash: Set(email_hash.to_string()),
        attempted_at: Set(Utc::now().into()),
        ..Default::default()
    };
    Ok(active_model.insert(db).await?)
}

/// Latest attempt for an email, if any. Used for the min-interval rate-limit
/// check ("no new request within N seconds of the previous one").
pub async fn find_most_recent(
    db: &impl ConnectionTrait,
    email_hash: &str,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::EmailHash.eq(email_hash))
        .order_by_desc(Column::AttemptedAt)
        .one(db)
        .await?)
}

/// Count attempts for an email since `since`. Used for the daily-cap rate
/// limit ("no more than N requests in the last 24h").
///
/// Implemented as `.all().len()` rather than SQL `COUNT(*)` for two reasons:
/// (1) the upper bound is the rate-limit cap itself (5 per 24h), so the
/// returned row count is tiny in practice; (2) it makes the function
/// straightforward to mock against `MockDatabase` in unit tests. The
/// composite `(email_hash, attempted_at DESC)` index keeps the query cheap.
pub async fn count_since(
    db: &impl ConnectionTrait,
    email_hash: &str,
    since: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    let rows = Entity::find()
        .filter(Column::EmailHash.eq(email_hash))
        .filter(Column::AttemptedAt.gte(since))
        .all(db)
        .await?;
    Ok(rows.len() as u64)
}

/// Delete attempts older than `cutoff` across all email hashes. Returns
/// the number of rows deleted.
///
/// Intended for periodic maintenance — see `domain::password_reset::sweep_old_attempts`
/// for the recommended wrapper, retention policy, and call pattern.
///
/// **Safe to call concurrently** with `record()`: under MVCC, a concurrent
/// INSERT with `attempted_at = NOW()` falls outside the `< cutoff` predicate
/// and is unaffected.
///
/// For ad-hoc inspection ("how many rows would I delete?") run the equivalent
/// `SELECT count(*) FROM refactor_platform.password_reset_attempts
///  WHERE attempted_at < <cutoff>` via psql — there is no dry-run mode here.
pub async fn delete_older_than(
    db: &impl ConnectionTrait,
    cutoff: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    let result = Entity::delete_many()
        .filter(Column::AttemptedAt.lt(cutoff))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}
