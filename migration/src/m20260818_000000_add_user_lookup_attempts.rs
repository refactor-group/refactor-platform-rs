use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Rate-limiter state for user-lookup requests, keyed by the requester.
        //
        // NOT append-only, unlike the near-identical `user_role_changes`: the
        // retention sweep deletes from here, so no UPDATE/DELETE/TRUNCATE revoke.
        //
        // No foreign key on `requester_user_id`, matching `password_reset_attempts`
        // and `user_role_changes`.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.user_lookup_attempts (
                    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    requester_user_id UUID NOT NULL,
                    attempted_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.user_lookup_attempts OWNER TO refactor",
            )
            .await?;

        // Serves the windowed count per requester, and the sweep's range scan.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_user_lookup_attempts_requester_time \
                 ON refactor_platform.user_lookup_attempts (requester_user_id, attempted_at DESC)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.user_lookup_attempts")
            .await?;
        Ok(())
    }
}
