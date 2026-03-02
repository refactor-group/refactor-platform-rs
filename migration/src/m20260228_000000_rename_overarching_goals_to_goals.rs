use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Rename the table
        conn.execute_unprepared("ALTER TABLE refactor_platform.overarching_goals RENAME TO goals")
            .await?;

        // Ensure ownership is set correctly for the refactor user
        conn.execute_unprepared("ALTER TABLE refactor_platform.goals OWNER TO refactor")
            .await?;

        // Rename sorting indexes to match new table name
        conn.execute_unprepared(
            "ALTER INDEX refactor_platform.overarching_goals_title RENAME TO goals_title",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER INDEX refactor_platform.overarching_goals_created_at RENAME TO goals_created_at",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER INDEX refactor_platform.overarching_goals_updated_at RENAME TO goals_updated_at",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Rename the table back
        conn.execute_unprepared("ALTER TABLE refactor_platform.goals RENAME TO overarching_goals")
            .await?;

        // Restore ownership
        conn.execute_unprepared(
            "ALTER TABLE refactor_platform.overarching_goals OWNER TO refactor",
        )
        .await?;

        // Rename indexes back
        conn.execute_unprepared(
            "ALTER INDEX refactor_platform.goals_title RENAME TO overarching_goals_title",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER INDEX refactor_platform.goals_created_at RENAME TO overarching_goals_created_at",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER INDEX refactor_platform.goals_updated_at RENAME TO overarching_goals_updated_at",
        )
        .await?;

        Ok(())
    }
}
