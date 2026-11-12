use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Topic order in the session is the only prioritisation now.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_topics DROP COLUMN IF EXISTS priority",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.topic_priority")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Restores the shape only; dropped values are not recoverable.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.topic_priority AS ENUM ('low', 'medium', 'high')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.topic_priority OWNER TO refactor")
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_topics \
                 ADD COLUMN priority refactor_platform.topic_priority",
            )
            .await?;

        Ok(())
    }
}
