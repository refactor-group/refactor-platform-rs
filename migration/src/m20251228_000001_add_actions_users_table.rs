use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create the actions_users junction table for many-to-many relationship
        // between actions and users. This allows assigning one or more users
        // (coach and/or coachee) to an action.
        //
        // We use execute_unprepared() for consistency with other migrations and to ensure
        // proper PostgreSQL schema qualification (refactor_platform.actions_users)
        let create_table_sql = "CREATE TABLE IF NOT EXISTS refactor_platform.actions_users (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            action_id UUID NOT NULL,
            user_id UUID NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            CONSTRAINT fk_actions_users_action
                FOREIGN KEY (action_id)
                REFERENCES refactor_platform.actions(id)
                ON DELETE CASCADE
                ON UPDATE CASCADE,
            CONSTRAINT fk_actions_users_user
                FOREIGN KEY (user_id)
                REFERENCES refactor_platform.users(id)
                ON DELETE CASCADE
                ON UPDATE CASCADE
        )";

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        // Set table ownership to refactor user to avoid permission issues
        // when migrations run as a different user (e.g., superuser like doadmin)
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE refactor_platform.actions_users OWNER TO refactor")
            .await?;

        // Create unique index to prevent duplicate assignments of the same user to the same action
        let create_unique_index_sql =
            "CREATE UNIQUE INDEX IF NOT EXISTS actions_users_action_user_unique
            ON refactor_platform.actions_users(action_id, user_id)";

        manager
            .get_connection()
            .execute_unprepared(create_unique_index_sql)
            .await?;

        // Create index on action_id for efficient querying of assignees by action
        let create_action_index_sql = "CREATE INDEX IF NOT EXISTS actions_users_action_id_idx
            ON refactor_platform.actions_users(action_id)";

        manager
            .get_connection()
            .execute_unprepared(create_action_index_sql)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the actions_users table (this will also drop the indexes and foreign keys)
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.actions_users")
            .await?;

        Ok(())
    }
}
