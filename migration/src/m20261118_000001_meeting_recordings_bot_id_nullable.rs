use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A recording claims its session before its bot exists, so the bot id comes later.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.meeting_recordings
                 ALTER COLUMN bot_id DROP NOT NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The row's own id keeps the bot id unique.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.meeting_recordings
                 SET bot_id = id::text
                 WHERE bot_id IS NULL",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.meeting_recordings
                 ALTER COLUMN bot_id SET NOT NULL",
            )
            .await?;

        Ok(())
    }
}
