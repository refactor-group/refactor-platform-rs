use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // When the participants were last told this session's start: at booking, and
        // again on every reschedule. The reminder sweep measures notice from here rather
        // than from `created_at`, so a session moved to a time less than the lead away
        // is not reminded about, the reschedule email having just said the same thing.
        //
        // Not `updated_at`: hydration and title edits move that without telling anyone,
        // which would silently eat the notice a reminder is owed.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN notice_given_at TIMESTAMPTZ NOT NULL DEFAULT NOW()",
            )
            .await?;

        // Existing rows were last announced when they were booked. Any reschedule since
        // is unrecorded, which errs toward reminding rather than toward silence.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_sessions SET notice_given_at = created_at",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS notice_given_at",
            )
            .await?;

        Ok(())
    }
}
