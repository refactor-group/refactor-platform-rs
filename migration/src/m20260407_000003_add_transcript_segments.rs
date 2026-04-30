use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Immutable speaker-diarized utterances. No updated_at — segments are write-once.
        // sentiment is VARCHAR (not enum) to avoid PostgreSQL enum ownership friction.
        let create_table_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.transcript_segments (
                id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                transcription_id UUID NOT NULL
                    REFERENCES refactor_platform.transcriptions(id) ON DELETE CASCADE,
                speaker_label    VARCHAR(255) NOT NULL,
                text             TEXT NOT NULL,
                start_ms         INTEGER NOT NULL,
                end_ms           INTEGER NOT NULL,
                confidence       DOUBLE PRECISION,
                sentiment        VARCHAR(20),
                created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.transcript_segments OWNER TO refactor",
            )
            .await?;

        // Supports ordered fetch for conversation UI
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_transcript_segments_transcription_start \
                 ON refactor_platform.transcript_segments(transcription_id, start_ms ASC)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.transcript_segments")
            .await?;

        Ok(())
    }
}
