use sea_orm_migration::prelude::*;

/// Default reminder lead time, mirrored from `SESSION_REMINDER_LEAD_HOURS`. Used only
/// by the backfill below; the running job reads the configured value.
const DEFAULT_LEAD_HOURS: i64 = 24;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The reminder sweep's idempotency key. Holds the `date` value the reminder
        // was sent for, rather than a plain "sent" timestamp, so a reschedule
        // re-arms the reminder without any other code path having to clear it:
        // the sweep's predicate is `reminder_sent_for_start IS DISTINCT FROM date`.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN reminder_sent_for_start TIMESTAMP",
            )
            .await?;

        // Suppress a deploy-time blast. Every session already inside the reminder
        // window (and every past one) is marked as though its reminder had gone out,
        // so enabling the job does not email every coachee with a session in the next
        // 24 hours the moment the container starts. Sessions further out are left
        // NULL and get a reminder on schedule.
        manager
            .get_connection()
            .execute_unprepared(&format!(
                "UPDATE refactor_platform.coaching_sessions
                 SET reminder_sent_for_start = date
                 WHERE date <= NOW() AT TIME ZONE 'UTC' + INTERVAL '{DEFAULT_LEAD_HOURS} hours'"
            ))
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS reminder_sent_for_start",
            )
            .await?;

        Ok(())
    }
}
