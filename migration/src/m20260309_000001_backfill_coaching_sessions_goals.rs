use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Backfill join table from existing created_in_session_id links.
        // Every goal that was created within a session context gets an
        // initial coaching_sessions_goals row so the relationship is
        // preserved in the new many-to-many structure.
        db.execute_unprepared(
            "INSERT INTO refactor_platform.coaching_sessions_goals
                (coaching_session_id, goal_id)
             SELECT created_in_session_id, id
             FROM refactor_platform.goals
             WHERE created_in_session_id IS NOT NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Before the schema migration drops the join table, recover
        // session links back into created_in_session_id for goals that
        // were linked via the join table but have a NULL created_in_session_id
        // (i.e. goals created after PR2 and linked explicitly).
        // Uses the earliest linked session (MIN) as a best-effort recovery.
        db.execute_unprepared(
            "UPDATE refactor_platform.goals g
             SET created_in_session_id = sub.coaching_session_id
             FROM (
                 SELECT goal_id, MIN(coaching_session_id) AS coaching_session_id
                 FROM refactor_platform.coaching_sessions_goals
                 GROUP BY goal_id
             ) sub
             WHERE g.id = sub.goal_id
             AND g.created_in_session_id IS NULL",
        )
        .await?;

        // Clear all rows from the join table so the schema migration's
        // down can drop it cleanly. (The schema migration owns the table.)
        db.execute_unprepared("DELETE FROM refactor_platform.coaching_sessions_goals")
            .await?;

        Ok(())
    }
}
