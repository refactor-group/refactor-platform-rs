use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// GIN expression indexes backing keyword search (`GET /search`, PR 1 of
/// docs/implementation-plans/search-capability-backend.md).
///
/// Each index expression must stay semantically identical to the tsvector form
/// of the matching `TEXT_EXPR` constant in `entity_api/src/search/`, or
/// Postgres will stop using the index for that searcher's query. Plain `CREATE INDEX` (not
/// CONCURRENTLY — migrations run in a transaction) is acceptable because every
/// phase-1 table is small.
const CREATE_INDEXES_SQL: &str = r#"
    CREATE INDEX IF NOT EXISTS idx_coaching_sessions_title_fts ON refactor_platform.coaching_sessions
      USING GIN (to_tsvector('english', coalesce(title, '')));
    CREATE INDEX IF NOT EXISTS idx_goals_fts ON refactor_platform.goals
      USING GIN (to_tsvector('english', coalesce(title,'') || ' ' || coalesce(body,'')));
    CREATE INDEX IF NOT EXISTS idx_actions_body_fts ON refactor_platform.actions
      USING GIN (to_tsvector('english', coalesce(body, '')));
    CREATE INDEX IF NOT EXISTS idx_agreements_body_fts ON refactor_platform.agreements
      USING GIN (to_tsvector('english', coalesce(body, '')));
    CREATE INDEX IF NOT EXISTS idx_coaching_session_topics_body_fts ON refactor_platform.coaching_session_topics
      USING GIN (to_tsvector('english', coalesce(body, '')));
"#;

const DROP_INDEXES_SQL: &str = r#"
    DROP INDEX IF EXISTS refactor_platform.idx_coaching_sessions_title_fts;
    DROP INDEX IF EXISTS refactor_platform.idx_goals_fts;
    DROP INDEX IF EXISTS refactor_platform.idx_actions_body_fts;
    DROP INDEX IF EXISTS refactor_platform.idx_agreements_body_fts;
    DROP INDEX IF EXISTS refactor_platform.idx_coaching_session_topics_body_fts;
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
