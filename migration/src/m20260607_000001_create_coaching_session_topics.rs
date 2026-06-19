use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Topics authored by a participant under a coaching session, with a
        // body and an author-controlled display_order.
        //
        // We use execute_unprepared() for consistency with other migrations and to ensure
        // proper PostgreSQL schema qualification (refactor_platform.coaching_session_topics)
        let create_table_sql =
            "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_session_topics (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            coaching_session_id UUID NOT NULL,
            user_id UUID NOT NULL,
            body TEXT NOT NULL,
            display_order INTEGER NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            CONSTRAINT fk_coaching_session_topics_session
                FOREIGN KEY (coaching_session_id)
                REFERENCES refactor_platform.coaching_sessions(id)
                ON DELETE CASCADE ON UPDATE CASCADE,
            CONSTRAINT fk_coaching_session_topics_user
                FOREIGN KEY (user_id)
                REFERENCES refactor_platform.users(id)
                ON DELETE CASCADE ON UPDATE CASCADE
        )";

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        // Index for ordered fetches of a session's topics
        let create_index_sql =
            "CREATE INDEX IF NOT EXISTS coaching_session_topics_session_order_idx
            ON refactor_platform.coaching_session_topics(coaching_session_id, display_order)";

        manager
            .get_connection()
            .execute_unprepared(create_index_sql)
            .await?;

        // Set table ownership to refactor user to avoid permission issues
        // when migrations run as a different user (e.g., superuser like doadmin)
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_topics OWNER TO refactor",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the coaching_session_topics table (this also drops its index and foreign keys)
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.coaching_session_topics")
            .await?;

        Ok(())
    }
}
