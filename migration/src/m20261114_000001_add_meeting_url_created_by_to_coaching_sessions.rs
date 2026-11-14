use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Whose account made the meeting link; null whenever there is no link.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN meeting_url_created_by_user_id UUID
                 REFERENCES refactor_platform.users(id) ON DELETE RESTRICT",
            )
            .await?;

        // Every existing link came from the coach's account or was set by the coach.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_sessions cs
                 SET meeting_url_created_by_user_id = cr.coach_id
                 FROM refactor_platform.coaching_relationships cr
                 WHERE cr.id = cs.coaching_relationship_id
                   AND cs.meeting_url IS NOT NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS meeting_url_created_by_user_id",
            )
            .await?;

        Ok(())
    }
}
