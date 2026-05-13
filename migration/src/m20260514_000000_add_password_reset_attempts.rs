use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Append-only audit log of password-reset request attempts, used by
        // the rate-limiter. Decoupled from `magic_link_tokens` because that
        // table is a *state* table (one live token per user/purpose, enforced
        // via delete-then-create) — counting rows in it would always return
        // 0 or 1, making the daily cap unreachable. See PR #311 for context.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.password_reset_attempts (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    user_id UUID NOT NULL REFERENCES refactor_platform.users(id) ON DELETE CASCADE,
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

        // Composite index on (user_id, attempted_at DESC) supports both
        // rate-limit queries:
        //   - find_most_recent(user_id)         → LIMIT 1 with ORDER BY
        //   - count_since(user_id, since)       → range scan from the head
        // The same index also accelerates ops sweeps within a single user.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_pwr_attempts_user_time \
                 ON refactor_platform.password_reset_attempts (user_id, attempted_at DESC)",
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
