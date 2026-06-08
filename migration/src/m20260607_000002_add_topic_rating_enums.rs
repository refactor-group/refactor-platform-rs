use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Coachee-set rating axes. Both default to 'neutral' (untriaged).
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.topic_relevance AS ENUM \
                 ('neutral', 'peripheral', 'worth_exploring', 'central')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.topic_relevance OWNER TO refactor")
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.topic_immediacy AS ENUM \
                 ('neutral', 'can_wait', 'soon', 'pressing')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.topic_immediacy OWNER TO refactor")
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_topics \
                 ADD COLUMN relevance refactor_platform.topic_relevance NOT NULL DEFAULT 'neutral', \
                 ADD COLUMN immediacy refactor_platform.topic_immediacy NOT NULL DEFAULT 'neutral'",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_topics \
                 DROP COLUMN immediacy, DROP COLUMN relevance",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.topic_immediacy")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.topic_relevance")
            .await?;

        Ok(())
    }
}
