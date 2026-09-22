use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Metadata for images pasted into a session's Coaching Notes. The bytes live in
        // object storage; storage_key is the only pointer to them and is unique per object.
        //
        // We use execute_unprepared() for consistency with other migrations and to ensure
        // proper PostgreSQL schema qualification (refactor_platform.coaching_session_images)
        //
        // The session FK cascades on delete deliberately: a future cleanup job can read a
        // deleted session's storage keys in one query before the rows disappear.
        let create_table_sql =
            "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_session_images (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            coaching_session_id UUID NOT NULL,
            uploaded_by_id UUID NOT NULL,
            storage_key TEXT NOT NULL UNIQUE,
            mime_type TEXT NOT NULL,
            byte_size BIGINT NOT NULL,
            width INTEGER,
            height INTEGER,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            CONSTRAINT fk_session_images_session
                FOREIGN KEY (coaching_session_id)
                REFERENCES refactor_platform.coaching_sessions(id)
                ON DELETE CASCADE ON UPDATE CASCADE,
            CONSTRAINT fk_session_images_user
                FOREIGN KEY (uploaded_by_id)
                REFERENCES refactor_platform.users(id)
                ON DELETE CASCADE ON UPDATE CASCADE
        )";

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        // Index for fetching every image belonging to one coaching session
        let create_index_sql = "CREATE INDEX IF NOT EXISTS coaching_session_images_session_idx
            ON refactor_platform.coaching_session_images(coaching_session_id)";

        manager
            .get_connection()
            .execute_unprepared(create_index_sql)
            .await?;

        // Set table ownership to refactor user to avoid permission issues
        // when migrations run as a different user (e.g., superuser like doadmin)
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_images OWNER TO refactor",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the coaching_session_images table (this also drops its index and foreign keys)
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.coaching_session_images")
            .await?;

        Ok(())
    }
}
