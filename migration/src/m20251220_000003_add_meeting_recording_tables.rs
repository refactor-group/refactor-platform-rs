use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create meeting_recording_status enum
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.meeting_recording_status AS ENUM (
                    'pending',
                    'joining',
                    'recording',
                    'processing',
                    'completed',
                    'failed'
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TYPE refactor_platform.meeting_recording_status OWNER TO refactor",
            )
            .await?;

        // Create transcription_status enum
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.transcription_status AS ENUM (
                    'pending',
                    'processing',
                    'completed',
                    'failed'
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TYPE refactor_platform.transcription_status OWNER TO refactor",
            )
            .await?;

        // Create sentiment enum for transcript segment analysis
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.sentiment AS ENUM (
                    'positive',
                    'neutral',
                    'negative'
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.sentiment OWNER TO refactor")
            .await?;

        // Create ai_suggestion_type enum
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.ai_suggestion_type AS ENUM (
                    'action',
                    'agreement'
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.ai_suggestion_type OWNER TO refactor")
            .await?;

        // Create ai_suggestion_status enum
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.ai_suggestion_status AS ENUM (
                    'pending',
                    'accepted',
                    'dismissed'
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TYPE refactor_platform.ai_suggestion_status OWNER TO refactor",
            )
            .await?;

        // Create meeting_recordings table
        let create_recordings_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.meeting_recordings (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                coaching_session_id UUID NOT NULL
                    REFERENCES refactor_platform.coaching_sessions(id) ON DELETE CASCADE,
                recall_bot_id VARCHAR(255),
                status refactor_platform.meeting_recording_status NOT NULL DEFAULT 'pending',
                recording_url TEXT,
                duration_seconds INTEGER,
                started_at TIMESTAMPTZ,
                ended_at TIMESTAMPTZ,
                error_message TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_recordings_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.meeting_recordings OWNER TO refactor",
            )
            .await?;

        // Create transcriptions table
        let create_transcriptions_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.transcriptions (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                meeting_recording_id UUID NOT NULL
                    REFERENCES refactor_platform.meeting_recordings(id) ON DELETE CASCADE,
                assemblyai_transcript_id VARCHAR(255),
                status refactor_platform.transcription_status NOT NULL DEFAULT 'pending',
                full_text TEXT,
                summary TEXT,
                confidence_score DOUBLE PRECISION,
                word_count INTEGER,
                language_code VARCHAR(10) DEFAULT 'en',
                error_message TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

                CONSTRAINT transcriptions_meeting_recording_unique UNIQUE(meeting_recording_id)
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_transcriptions_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE refactor_platform.transcriptions OWNER TO refactor")
            .await?;

        // Create transcript_segments table (utterances with speaker diarization)
        let create_segments_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.transcript_segments (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                transcription_id UUID NOT NULL
                    REFERENCES refactor_platform.transcriptions(id) ON DELETE CASCADE,
                speaker_label VARCHAR(50) NOT NULL,
                speaker_user_id UUID REFERENCES refactor_platform.users(id),
                text TEXT NOT NULL,
                start_time_ms BIGINT NOT NULL,
                end_time_ms BIGINT NOT NULL,
                confidence DOUBLE PRECISION,
                sentiment refactor_platform.sentiment,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_segments_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.transcript_segments OWNER TO refactor",
            )
            .await?;

        // Create ai_suggested_items table (before user approval)
        let create_suggestions_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.ai_suggested_items (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                transcription_id UUID NOT NULL
                    REFERENCES refactor_platform.transcriptions(id) ON DELETE CASCADE,
                item_type refactor_platform.ai_suggestion_type NOT NULL,
                content TEXT NOT NULL,
                source_text TEXT,
                confidence DOUBLE PRECISION,
                status refactor_platform.ai_suggestion_status NOT NULL DEFAULT 'pending',
                accepted_entity_id UUID,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_suggestions_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.ai_suggested_items OWNER TO refactor",
            )
            .await?;

        // Create indexes for efficient querying
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_meeting_recordings_session
                 ON refactor_platform.meeting_recordings(coaching_session_id)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_transcriptions_recording
                 ON refactor_platform.transcriptions(meeting_recording_id)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_transcript_segments_transcription
                 ON refactor_platform.transcript_segments(transcription_id)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_ai_suggested_items_transcription
                 ON refactor_platform.ai_suggested_items(transcription_id)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_ai_suggested_items_status
                 ON refactor_platform.ai_suggested_items(status)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop tables in reverse order of creation (respecting foreign key dependencies)
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.ai_suggested_items")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.transcript_segments")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.transcriptions")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.meeting_recordings")
            .await?;

        // Drop enum types
        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.ai_suggestion_status")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.ai_suggestion_type")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.sentiment")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.transcription_status")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.meeting_recording_status")
            .await?;

        Ok(())
    }
}
