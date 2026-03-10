use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Step 1: Add coaching_relationship_id as nullable (for backfill)
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             ADD COLUMN coaching_relationship_id UUID",
        )
        .await?;

        // Step 2: Backfill coaching_relationship_id from coaching_sessions
        db.execute_unprepared(
            "UPDATE refactor_platform.goals g
             SET coaching_relationship_id = cs.coaching_relationship_id
             FROM refactor_platform.coaching_sessions cs
             WHERE g.coaching_session_id = cs.id",
        )
        .await?;

        // Step 3: Make coaching_relationship_id NOT NULL now that all rows are backfilled
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             ALTER COLUMN coaching_relationship_id SET NOT NULL",
        )
        .await?;

        // Step 4: Add FK constraint for coaching_relationship_id
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             ADD CONSTRAINT fk_goals_coaching_relationship
             FOREIGN KEY (coaching_relationship_id)
             REFERENCES refactor_platform.coaching_relationships(id)
             ON DELETE CASCADE ON UPDATE CASCADE",
        )
        .await?;

        // Step 5: Add index on coaching_relationship_id for efficient querying
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS goals_coaching_relationship_id_idx
             ON refactor_platform.goals(coaching_relationship_id)",
        )
        .await?;

        // Step 6: Rename coaching_session_id → created_in_session_id
        // Column was nullable (uuid without NOT NULL) in the original base SQL schema,
        // even though the old Rust entity used non-optional Id. No constraint change needed.
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             RENAME COLUMN coaching_session_id TO created_in_session_id",
        )
        .await?;

        // Step 7: Add target_date column (nullable DATE)
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             ADD COLUMN target_date DATE",
        )
        .await?;

        // Step 8: Create the coaching_sessions_goals join table
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_sessions_goals (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                coaching_session_id UUID NOT NULL,
                goal_id UUID NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT fk_coaching_sessions_goals_session
                    FOREIGN KEY (coaching_session_id)
                    REFERENCES refactor_platform.coaching_sessions(id)
                    ON DELETE CASCADE
                    ON UPDATE CASCADE,
                CONSTRAINT fk_coaching_sessions_goals_goal
                    FOREIGN KEY (goal_id)
                    REFERENCES refactor_platform.goals(id)
                    ON DELETE CASCADE
                    ON UPDATE CASCADE
            )",
        )
        .await?;

        // Step 9: Set ownership to refactor user
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.coaching_sessions_goals OWNER TO refactor",
        )
        .await?;

        // Step 10: Unique index to prevent duplicate session-goal links
        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS coaching_sessions_goals_session_goal_unique
             ON refactor_platform.coaching_sessions_goals(coaching_session_id, goal_id)",
        )
        .await?;

        // Step 11: Index on goal_id for reverse lookups (sessions for a goal)
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS coaching_sessions_goals_goal_id_idx
             ON refactor_platform.coaching_sessions_goals(goal_id)",
        )
        .await?;

        // Step 12: Backfill join table from existing created_in_session_id links
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

        // Step 1: Before dropping the join table, backfill created_in_session_id NULLs
        // from the join table. Goals created after PR2 with NULL created_in_session_id
        // may have been linked to sessions via the join table — recover that link.
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

        // Step 2: Drop the join table (also drops its indexes and FKs)
        db.execute_unprepared("DROP TABLE IF EXISTS refactor_platform.coaching_sessions_goals")
            .await?;

        // Step 3: Remove target_date column
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             DROP COLUMN IF EXISTS target_date",
        )
        .await?;

        // Step 4: Rename created_in_session_id back to coaching_session_id
        // Column stays nullable (was nullable uuid in the original base SQL schema)
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             RENAME COLUMN created_in_session_id TO coaching_session_id",
        )
        .await?;

        // Step 5: Drop coaching_relationship_id index
        db.execute_unprepared(
            "DROP INDEX IF EXISTS refactor_platform.goals_coaching_relationship_id_idx",
        )
        .await?;

        // Step 6: Drop coaching_relationship_id FK constraint
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             DROP CONSTRAINT IF EXISTS fk_goals_coaching_relationship",
        )
        .await?;

        // Step 7: Remove coaching_relationship_id column
        db.execute_unprepared(
            "ALTER TABLE refactor_platform.goals
             DROP COLUMN IF EXISTS coaching_relationship_id",
        )
        .await?;

        Ok(())
    }
}
