use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // The original FK constraint (from base SQL schema) was created as:
        //   ALTER TABLE overarching_goals ADD FOREIGN KEY (coaching_session_id) REFERENCES coaching_sessions(id);
        // This defaulted to ON DELETE RESTRICT. When the column was renamed to
        // created_in_session_id, PostgreSQL kept the old constraint name and behavior.
        //
        // Since created_in_session_id is nullable and only records which session a goal
        // was created in, ON DELETE SET NULL is correct: deleting a session should null
        // out the reference, not block the delete.

        // Step 1: Drop the old FK constraint (still using the pre-rename name)
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             DROP CONSTRAINT IF EXISTS overarching_goals_coaching_session_id_fkey",
        )
        .await?;

        // Step 2: Re-add the FK with ON DELETE SET NULL and a properly named constraint
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             ADD CONSTRAINT fk_goals_created_in_session
             FOREIGN KEY (created_in_session_id)
             REFERENCES refactor_platform.coaching_sessions(id)
             ON DELETE SET NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Restore the original RESTRICT behavior
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             DROP CONSTRAINT IF EXISTS fk_goals_created_in_session",
        )
        .await?;

        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             ADD CONSTRAINT overarching_goals_coaching_session_id_fkey
             FOREIGN KEY (created_in_session_id)
             REFERENCES refactor_platform.coaching_sessions(id)",
        )
        .await?;

        Ok(())
    }
}
