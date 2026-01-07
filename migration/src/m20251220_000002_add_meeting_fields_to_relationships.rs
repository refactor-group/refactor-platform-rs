use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create the ai_privacy_level enum for per-relationship privacy settings
        // This allows coaches to configure AI features on a per-client basis:
        // - 'none': No AI recording or transcribing (for clients uncomfortable with AI)
        // - 'transcribe_only': Text transcription only, no video/audio storage
        // - 'full': All AI recording and transcribing features enabled
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.ai_privacy_level AS ENUM (
                    'none',
                    'transcribe_only',
                    'full'
                )",
            )
            .await?;

        // Set ownership to refactor user
        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.ai_privacy_level OWNER TO refactor")
            .await?;

        // Add meeting_url and ai_privacy_level columns to coaching_relationships
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_relationships
                 ADD COLUMN IF NOT EXISTS meeting_url VARCHAR(500),
                 ADD COLUMN IF NOT EXISTS ai_privacy_level refactor_platform.ai_privacy_level NOT NULL DEFAULT 'full'",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove the columns from coaching_relationships
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_relationships
                 DROP COLUMN IF EXISTS meeting_url,
                 DROP COLUMN IF EXISTS ai_privacy_level",
            )
            .await?;

        // Drop the enum type
        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.ai_privacy_level")
            .await?;

        Ok(())
    }
}
