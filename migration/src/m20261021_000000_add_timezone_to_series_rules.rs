use sea_orm_migration::prelude::*;

/// Records on each series rule the zone its invites were published in, so slots are
/// expanded in the same zone the calendar expands its `RRULE` in. Existing series take
/// the coach's current zone, which is the one every invite so far was written in.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A zone Postgres does not know falls back to UTC, as the invite builder does.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_session_series AS s
                 SET rule = jsonb_set(
                     s.rule,
                     '{timezone}',
                     to_jsonb(CASE
                         WHEN u.timezone IN (SELECT name FROM pg_timezone_names) THEN u.timezone
                         ELSE 'UTC'
                     END)
                 )
                 FROM refactor_platform.coaching_relationships AS r
                 JOIN refactor_platform.users AS u ON u.id = r.coach_id
                 WHERE r.id = s.coaching_relationship_id
                   AND NOT (s.rule ? 'timezone')",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_session_series
                 SET rule = rule - 'timezone'",
            )
            .await?;

        Ok(())
    }
}
