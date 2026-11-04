use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Partial GIN expression index backing transcript suggestion search (PR 1b of
/// docs/implementation-plans/search-capability-backend.md).
///
/// The expression must stay semantically identical to the tsvector form of
/// `TEXT_EXPR` in `entity_api/src/search/transcript_suggestion.rs`, and the
/// predicate to its `PENDING_PREDICATE`, or Postgres stops using the index.
/// Only pending suggestions are searchable, so only they are indexed.
const CREATE_INDEX_SQL: &str = r#"
    CREATE INDEX IF NOT EXISTS idx_transcript_suggestions_pending_fts ON refactor_platform.transcript_suggestions
      USING GIN (to_tsvector('english', coalesce(proposal ->> 'body', '')))
      WHERE status = 'pending' AND created_entity_id IS NULL;
"#;

const DROP_INDEX_SQL: &str = r#"
    DROP INDEX IF EXISTS refactor_platform.idx_transcript_suggestions_pending_fts;
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(CREATE_INDEX_SQL)
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(DROP_INDEX_SQL)
            .await?;
        Ok(())
    }
}
