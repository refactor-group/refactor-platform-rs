use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Settles any session written by code that didn't set it, on the same coach
        // assumption as the original backfill, so NOT NULL can be added.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_sessions cs
                 SET scheduled_by_user_id = cr.coach_id
                 FROM refactor_platform.coaching_relationships cr
                 WHERE cr.id = cs.coaching_relationship_id
                   AND cs.scheduled_by_user_id IS NULL",
            )
            .await?;

        // A link set by code that didn't record its creator came from the coach.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_sessions cs
                 SET meeting_url_created_by_user_id = cr.coach_id
                 FROM refactor_platform.coaching_relationships cr
                 WHERE cr.id = cs.coaching_relationship_id
                   AND cs.meeting_url IS NOT NULL
                   AND cs.meeting_url_created_by_user_id IS NULL",
            )
            .await?;

        // Every running writer now sets it.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ALTER COLUMN scheduled_by_user_id SET NOT NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ALTER COLUMN scheduled_by_user_id DROP NOT NULL",
            )
            .await?;

        Ok(())
    }
}
