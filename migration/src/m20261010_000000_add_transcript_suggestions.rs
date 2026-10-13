use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for statement in [
            "CREATE TYPE refactor_platform.transcript_suggestion_target AS ENUM ('action')",
            "ALTER TYPE refactor_platform.transcript_suggestion_target OWNER TO refactor",
            "CREATE TYPE refactor_platform.transcript_analysis_status AS ENUM ('pending', 'processing', 'completed', 'failed')",
            "ALTER TYPE refactor_platform.transcript_analysis_status OWNER TO refactor",
            "CREATE TYPE refactor_platform.transcript_suggestion_status AS ENUM ('pending', 'accepted', 'dismissed')",
            "ALTER TYPE refactor_platform.transcript_suggestion_status OWNER TO refactor",
        ] {
            db.execute_unprepared(statement).await?;
        }

        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.transcript_analyses (
                id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                transcription_id UUID NOT NULL
                    REFERENCES refactor_platform.transcriptions(id) ON DELETE CASCADE,
                target           refactor_platform.transcript_suggestion_target NOT NULL,
                status           refactor_platform.transcript_analysis_status NOT NULL DEFAULT 'pending',
                attempts         SMALLINT NOT NULL DEFAULT 0,
                claimed_at       TIMESTAMPTZ,
                error_message    TEXT,
                model            VARCHAR(255),
                input_tokens     INTEGER,
                output_tokens    INTEGER,
                created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                CONSTRAINT transcript_analyses_transcription_target_unique
                    UNIQUE (transcription_id, target)
            )
            "#,
        )
        .await?;
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.transcript_analyses OWNER TO refactor",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS transcript_analyses_status_idx \
             ON refactor_platform.transcript_analyses (status)",
        )
        .await?;

        // duplicate_of_id and created_entity_id point at the target's own table, so they carry no FK.
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.transcript_suggestions (
                id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                coaching_session_id    UUID NOT NULL
                    REFERENCES refactor_platform.coaching_sessions(id) ON DELETE CASCADE,
                transcript_analysis_id UUID NOT NULL
                    REFERENCES refactor_platform.transcript_analyses(id) ON DELETE CASCADE,
                target                 refactor_platform.transcript_suggestion_target NOT NULL,
                proposal               JSONB NOT NULL,
                segment_id             UUID NOT NULL
                    REFERENCES refactor_platform.transcript_segments(id) ON DELETE CASCADE,
                quote                  TEXT NOT NULL,
                duplicate_of_id        UUID,
                status                 refactor_platform.transcript_suggestion_status NOT NULL DEFAULT 'pending',
                resolved_by            UUID
                    REFERENCES refactor_platform.users(id) ON DELETE SET NULL,
                resolved_at            TIMESTAMPTZ,
                created_entity_id      UUID,
                created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .await?;
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.transcript_suggestions OWNER TO refactor",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS transcript_suggestions_session_target_status_idx \
             ON refactor_platform.transcript_suggestions (coaching_session_id, target, status)",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for statement in [
            "DROP TABLE IF EXISTS refactor_platform.transcript_suggestions",
            "DROP TABLE IF EXISTS refactor_platform.transcript_analyses",
            "DROP TYPE IF EXISTS refactor_platform.transcript_suggestion_status",
            "DROP TYPE IF EXISTS refactor_platform.transcript_analysis_status",
            "DROP TYPE IF EXISTS refactor_platform.transcript_suggestion_target",
        ] {
            db.execute_unprepared(statement).await?;
        }

        Ok(())
    }
}
