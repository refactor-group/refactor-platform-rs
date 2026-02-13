use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add meeting fields to coaching_sessions (not coaching_relationships).
        // Each session can optionally have its own meeting link.
        // Both columns are nullable — a session doesn't require a meeting.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN meeting_url VARCHAR(500),
                 ADD COLUMN provider refactor_platform.provider",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS meeting_url,
                 DROP COLUMN IF EXISTS provider",
            )
            .await?;

        Ok(())
    }
}
