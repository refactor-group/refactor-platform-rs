use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Per-(user, coaching_session) "last viewed at" marker. At most one
        // row per pair (UNIQUE), which both serves the upsert conflict target
        // and provides a btree for the per-user lookup. No extra index needed.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_session_views (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    user_id UUID NOT NULL REFERENCES refactor_platform.users(id) ON DELETE CASCADE,
                    coaching_session_id UUID NOT NULL REFERENCES refactor_platform.coaching_sessions(id) ON DELETE CASCADE,
                    last_viewed_at TIMESTAMPTZ NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    CONSTRAINT uq_coaching_session_views_user_session UNIQUE (user_id, coaching_session_id)
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_views OWNER TO refactor",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.coaching_session_views")
            .await?;
        Ok(())
    }
}
