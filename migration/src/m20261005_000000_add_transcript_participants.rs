use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            "CREATE TYPE refactor_platform.speaker_match_source AS ENUM ('account', 'elimination')",
        )
        .await?;
        db.execute_unprepared(
            "ALTER TYPE refactor_platform.speaker_match_source OWNER TO refactor",
        )
        .await?;

        // One-directional CHECK: an attributed user has a source, but deleting the user (SET NULL) must still succeed.
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.transcript_participants (
                id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                transcription_id        UUID NOT NULL
                    REFERENCES refactor_platform.transcriptions(id) ON DELETE CASCADE,
                provider_participant_id VARCHAR(255) NOT NULL,
                display_name            VARCHAR(255),
                is_host                 BOOLEAN,
                platform                VARCHAR(64),
                platform_account_id     VARCHAR(255),
                extra_data              JSONB,
                user_id                 UUID
                    REFERENCES refactor_platform.users(id) ON DELETE SET NULL,
                match_source            refactor_platform.speaker_match_source,
                created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                CONSTRAINT transcript_participants_transcription_provider_unique
                    UNIQUE (transcription_id, provider_participant_id),
                CONSTRAINT transcript_participants_user_has_source
                    CHECK (user_id IS NULL OR match_source IS NOT NULL)
            )
            "#,
        )
        .await?;
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.transcript_participants OWNER TO refactor",
        )
        .await?;

        db.execute_unprepared(
            "ALTER TABLE refactor_platform.transcript_segments \
             ADD COLUMN IF NOT EXISTS participant_id UUID \
             REFERENCES refactor_platform.transcript_participants(id) ON DELETE SET NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            "ALTER TABLE refactor_platform.transcript_segments DROP COLUMN IF EXISTS participant_id",
        )
        .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS refactor_platform.transcript_participants")
            .await?;
        db.execute_unprepared("DROP TYPE IF EXISTS refactor_platform.speaker_match_source")
            .await?;

        Ok(())
    }
}
