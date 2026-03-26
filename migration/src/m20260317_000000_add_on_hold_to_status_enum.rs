use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TYPE refactor_platform.status ADD VALUE IF NOT EXISTS 'on_hold'",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.status OWNER TO refactor")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Move any on_hold goals/actions back to not_started
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.goals SET status = 'not_started' WHERE status = 'on_hold'",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.actions SET status = 'not_started' WHERE status = 'on_hold'",
            )
            .await?;

        // 2. Rename the old enum
        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.status RENAME TO status_old")
            .await?;

        // 3. Create a new enum without on_hold
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.status AS ENUM ('not_started', 'in_progress', 'completed', 'wont_do')",
            )
            .await?;

        // 4. Update goals to use the new enum
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.goals
                ALTER COLUMN status TYPE refactor_platform.status
                USING status::text::refactor_platform.status",
            )
            .await?;

        // 5. Update actions to use the new enum
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.actions
                ALTER COLUMN status TYPE refactor_platform.status
                USING status::text::refactor_platform.status",
            )
            .await?;

        // 6. Drop the old enum
        manager
            .get_connection()
            .execute_unprepared("DROP TYPE refactor_platform.status_old")
            .await?;

        // 7. Restore ownership
        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.status OWNER TO refactor")
            .await?;

        Ok(())
    }
}
