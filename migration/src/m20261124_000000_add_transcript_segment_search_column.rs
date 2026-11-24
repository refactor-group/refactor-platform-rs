use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Stored tsvector column and GIN index backing transcript search (PR 2 of
/// docs/implementation-plans/search-capability-backend.md).
///
/// A stored column rather than the expression index the other searched tables
/// use: `transcript_segments` is the largest of them, and with an expression
/// index every candidate row's text is re-parsed to match and rank it. The
/// searcher reads the column as `TSV_EXPR` in
/// `entity_api/src/search/transcript.rs`.
///
/// Adding the column rewrites the table under an exclusive lock, so transcript
/// reads and writes wait while this migration runs.
const UP_SQL: &str = r#"
    ALTER TABLE refactor_platform.transcript_segments
      ADD COLUMN IF NOT EXISTS tsv tsvector
      GENERATED ALWAYS AS (to_tsvector('english', text)) STORED;
    CREATE INDEX IF NOT EXISTS idx_transcript_segments_tsv ON refactor_platform.transcript_segments
      USING GIN (tsv);
"#;

const DOWN_SQL: &str = r#"
    DROP INDEX IF EXISTS refactor_platform.idx_transcript_segments_tsv;
    ALTER TABLE refactor_platform.transcript_segments DROP COLUMN IF EXISTS tsv;
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(UP_SQL).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(DOWN_SQL)
            .await?;
        Ok(())
    }
}
