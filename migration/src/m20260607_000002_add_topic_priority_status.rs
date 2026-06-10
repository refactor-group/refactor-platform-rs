use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Coachee-set priority. Nullable: unset until the coachee triages.
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

        // Lifecycle status. NOT NULL, defaults to 'open'.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.topic_status AS ENUM ('open', 'discussed', 'deferred')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.topic_status OWNER TO refactor")
            .await?;

        // priority is nullable (no default); status defaults to 'open';
        // moved_from_session_id is a nullable FK to the session a topic was moved out of.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_topics \
                 ADD COLUMN priority refactor_platform.topic_priority, \
                 ADD COLUMN status refactor_platform.topic_status NOT NULL DEFAULT 'open', \
                 ADD COLUMN moved_from_session_id UUID \
                 REFERENCES refactor_platform.coaching_sessions(id) ON DELETE SET NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_topics \
                 DROP COLUMN moved_from_session_id, DROP COLUMN status, DROP COLUMN priority",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.topic_status")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.topic_priority")
            .await?;

        Ok(())
    }
}
