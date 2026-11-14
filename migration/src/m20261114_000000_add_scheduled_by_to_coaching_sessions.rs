use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Who last created or rescheduled the session. Nullable while code that doesn't
        // write it may still be running, so its inserts still succeed.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN scheduled_by_user_id UUID
                 REFERENCES refactor_platform.users(id) ON DELETE RESTRICT",
            )
            .await?;

        // Lets a user delete's RESTRICT check find sessions without a full scan.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS coaching_sessions_scheduled_by_idx
                 ON refactor_platform.coaching_sessions(scheduled_by_user_id)",
            )
            .await?;

        // Assumed to be the coach: the app's screens have only let the coach create or
        // reschedule a session, though the API let any signed-in user create one until Nov 10.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_sessions cs
                 SET scheduled_by_user_id = cr.coach_id
                 FROM refactor_platform.coaching_relationships cr
                 WHERE cr.id = cs.coaching_relationship_id",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS scheduled_by_user_id",
            )
            .await?;

        Ok(())
    }
}
