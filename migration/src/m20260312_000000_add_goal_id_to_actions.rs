use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Step 1: Add nullable goal_id column to actions
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.actions
             ADD COLUMN goal_id UUID",
        )
        .await?;

        // Step 2: Add FK constraint with ON DELETE SET NULL
        // When a goal is deleted, actions keep their data but lose the goal link
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.actions
             ADD CONSTRAINT fk_actions_goal
             FOREIGN KEY (goal_id)
             REFERENCES refactor_platform.goals(id)
             ON DELETE SET NULL ON UPDATE CASCADE",
        )
        .await?;

        // Step 3: Add index for efficient querying by goal_id
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS actions_goal_id_idx
             ON refactor_platform.actions(goal_id)",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            "ALTER TABLE refactor_platform.actions
             DROP CONSTRAINT IF EXISTS fk_actions_goal",
        )
        .await?;

        db.execute_unprepared("DROP INDEX IF EXISTS refactor_platform.actions_goal_id_idx")
            .await?;

        db.execute_unprepared(
            "ALTER TABLE refactor_platform.actions
             DROP COLUMN IF EXISTS goal_id",
        )
        .await?;

        Ok(())
    }
}
