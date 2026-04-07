use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.transcription_status AS ENUM \
                 ('queued', 'processing', 'completed', 'failed')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TYPE refactor_platform.transcription_status OWNER TO refactor",
            )
            .await?;

        // Stores Recall.ai async transcript metadata. Full content lives in transcript_segments.
        let create_table_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.transcriptions (
                id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                coaching_session_id  UUID NOT NULL
                    REFERENCES refactor_platform.coaching_sessions(id) ON DELETE CASCADE,
                meeting_recording_id UUID NOT NULL
                    REFERENCES refactor_platform.meeting_recordings(id) ON DELETE CASCADE,
                external_id          VARCHAR(255) NOT NULL,
                status               refactor_platform.transcription_status NOT NULL DEFAULT 'queued',
                language_code        VARCHAR(20),
                speaker_count        SMALLINT,
                word_count           INTEGER,
                duration_seconds     INTEGER,
                confidence           DOUBLE PRECISION,
                error_message        TEXT,
                created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.transcriptions OWNER TO refactor",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_transcriptions_coaching_session_id \
                 ON refactor_platform.transcriptions(coaching_session_id)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_transcriptions_external_id \
                 ON refactor_platform.transcriptions(external_id)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.transcriptions")
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "DROP TYPE IF EXISTS refactor_platform.transcription_status",
            )
            .await?;

        Ok(())
    }
}
