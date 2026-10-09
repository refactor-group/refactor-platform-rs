use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// Both back `ON DELETE SET NULL` foreign keys, which Postgres does not index itself.
const CREATE_INDEXES_SQL: &str = r#"
    CREATE INDEX IF NOT EXISTS idx_transcript_segments_participant_id
    ON refactor_platform.transcript_segments(participant_id);
    CREATE INDEX IF NOT EXISTS idx_transcript_participants_user_id
    ON refactor_platform.transcript_participants(user_id);
"#;

const DROP_INDEXES_SQL: &str = r#"
    DROP INDEX IF EXISTS refactor_platform.idx_transcript_segments_participant_id;
    DROP INDEX IF EXISTS refactor_platform.idx_transcript_participants_user_id;
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(CREATE_INDEXES_SQL)
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(DROP_INDEXES_SQL)
            .await?;
        Ok(())
    }
}
