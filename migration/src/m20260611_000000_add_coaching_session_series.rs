use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A coaching_session_series owns a recurrence rule and groups the
        // materialized coaching_sessions that were created from it. The rule
        // is stored as JSONB so the shape can evolve without a schema change.
        //
        // ON DELETE CASCADE from coaching_relationships matches the existing
        // policy on coaching_sessions: deleting the relationship purges all
        // dependent rows.
        //
        // ON DELETE RESTRICT from users on created_by_user_id ensures we
        // can't accidentally orphan a series by deleting its creator.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_session_series (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    coaching_relationship_id UUID NOT NULL
                        REFERENCES refactor_platform.coaching_relationships(id) ON DELETE CASCADE,
                    rule JSONB NOT NULL,
                    created_by_user_id UUID NOT NULL
                        REFERENCES refactor_platform.users(id) ON DELETE RESTRICT,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_series OWNER TO refactor",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS coaching_session_series_relationship_idx
                 ON refactor_platform.coaching_session_series(coaching_relationship_id)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN coaching_session_series_id UUID
                 REFERENCES refactor_platform.coaching_session_series(id) ON DELETE SET NULL",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS coaching_sessions_series_idx
                 ON refactor_platform.coaching_sessions(coaching_session_series_id)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX IF EXISTS refactor_platform.coaching_sessions_series_idx",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS coaching_session_series_id",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "DROP TABLE IF EXISTS refactor_platform.coaching_session_series",
            )
            .await?;

        Ok(())
    }
}
