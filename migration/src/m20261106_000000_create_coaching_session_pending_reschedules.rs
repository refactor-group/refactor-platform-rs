use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One row per session with an unannounced schedule change; no row means
        // participants know the current schedule. Times are naive UTC like `date`.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_session_pending_reschedules (
                    coaching_session_id UUID PRIMARY KEY
                        REFERENCES refactor_platform.coaching_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE,
                    announced_date TIMESTAMP NOT NULL,
                    announced_duration_minutes SMALLINT NOT NULL,
                    unannounced_since TIMESTAMP NOT NULL,
                    changed_at TIMESTAMP NOT NULL,
                    revision INTEGER NOT NULL DEFAULT 0,
                    attempts SMALLINT NOT NULL DEFAULT 0,
                    claim_id UUID NULL,
                    claimed_at TIMESTAMP NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS coaching_session_pending_reschedules_unannounced_since
                    ON refactor_platform.coaching_session_pending_reschedules (unannounced_since)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_pending_reschedules OWNER TO refactor",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP TABLE IF EXISTS refactor_platform.coaching_session_pending_reschedules",
            )
            .await?;
        Ok(())
    }
}
