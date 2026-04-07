use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.meeting_recording_status AS ENUM \
                 ('pending', 'joining', 'waiting_room', 'in_meeting', 'recording', \
                  'processing', 'completed', 'failed')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TYPE refactor_platform.meeting_recording_status OWNER TO refactor",
            )
            .await?;

        // Records one Recall.ai bot per coaching session attempt.
        // Multiple rows may exist per session (retries after failure).
        let create_table_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.meeting_recordings (
                id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                coaching_session_id UUID NOT NULL
                    REFERENCES refactor_platform.coaching_sessions(id) ON DELETE CASCADE,
                bot_id              VARCHAR(255) NOT NULL,
                status              refactor_platform.meeting_recording_status NOT NULL DEFAULT 'pending',
                video_url           TEXT,
                audio_url           TEXT,
                duration_seconds    INTEGER,
                started_at          TIMESTAMPTZ,
                ended_at            TIMESTAMPTZ,
                error_message       TEXT,
                created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.meeting_recordings OWNER TO refactor",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_meeting_recordings_coaching_session_id \
                 ON refactor_platform.meeting_recordings(coaching_session_id, created_at DESC)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.meeting_recordings")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.meeting_recording_status")
            .await?;

        Ok(())
    }
}
