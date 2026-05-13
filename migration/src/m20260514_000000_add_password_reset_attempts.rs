use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Append-only audit log of password-reset request attempts, used by
        // the rate-limiter.
        //
        // Keyed by SHA-256 of the normalized email (lowercased, trimmed) —
        // NOT by `user_id` — so the rate limit applies *uniformly* whether
        // or not the email maps to a real user. Keying by `user_id` would
        // mean the unknown-email path skips the rate-limit check, and the
        // resulting 200/429 asymmetry on subsequent requests is itself an
        // enumeration oracle (PR #311 review caught this).
        //
        // Decoupled from `magic_link_tokens` because that is a *state*
        // table (one live token per user/purpose, enforced via delete-then-
        // create) — counting rows in it would always return 0 or 1, making
        // the daily cap unreachable. The audit table preserves history.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.password_reset_attempts (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    email_hash VARCHAR(64) NOT NULL,
                    attempted_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.password_reset_attempts OWNER TO refactor",
            )
            .await?;

        // Composite index on (email_hash, attempted_at DESC) supports both
        // rate-limit queries:
        //   - find_most_recent(email_hash)        → LIMIT 1 with ORDER BY
        //   - count_since(email_hash, since)      → range scan from the head
        // No index on `attempted_at` alone; the sweep job's full-table
        // DELETE is acceptable because it runs off-peak (nightly).
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_pwr_attempts_email_time \
                 ON refactor_platform.password_reset_attempts (email_hash, attempted_at DESC)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.password_reset_attempts")
            .await?;
        Ok(())
    }
}
