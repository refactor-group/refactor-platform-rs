use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Per-coach default session duration. The column is universal across all
        // users (mirrors the `timezone` pattern — a per-user setting whose value
        // every row has, even when its meaning is role-dependent).
        //
        // SMALLINT matches `coaching_sessions.duration_minutes`. DEFAULT 60
        // backfills existing rows in a single column-add.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.users
                 ADD COLUMN default_coaching_session_duration_minutes SMALLINT NOT NULL DEFAULT 60",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.users
                 DROP COLUMN IF EXISTS default_coaching_session_duration_minutes",
            )
            .await?;

        Ok(())
    }
}
