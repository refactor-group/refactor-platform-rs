use sea_orm_migration::prelude::*;

/// Fallback reminder lead time when `SESSION_REMINDER_LEAD_HOURS` is unset, mirroring
/// the running job's own default.
const DEFAULT_LEAD_HOURS: i64 = 24;

/// Upper bound on the suppression window, well past any real lead time.
const MAX_SUPPRESSION_HOURS: u64 = 1_000_000;

/// The lead time the backfill should suppress, read from the same variable the job uses.
///
/// Suppressing a different window than the job sweeps is wrong in both directions: too
/// narrow and the first sweep mails everyone in the gap at once, too wide and sessions in
/// the gap are marked as reminded without an email ever being sent.
fn suppression_lead_hours() -> i64 {
    std::env::var("SESSION_REMINDER_LEAD_HOURS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        // `.max(1)` mirrors the runtime's own clamp, so a configured `0` suppresses the
        // hour the job will actually sweep rather than a whole day it never will. The
        // upper bound only keeps the interval literal inside what Postgres accepts.
        .map(|hours| hours.clamp(1, MAX_SUPPRESSION_HOURS) as i64)
        .unwrap_or(DEFAULT_LEAD_HOURS)
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The reminder sweep's idempotency key, one row per (session, recipient).
        // `sent_for_start` holds the `date` the reminder was sent for rather than a
        // plain "sent at" timestamp, so a reschedule re-arms the reminder without any
        // other code path having to clear it: the sweep claims pairs whose stored
        // value `IS DISTINCT FROM` the session's current start.
        //
        // Keyed by recipient rather than by session alone so a second recipient can be
        // added later without migrating stamped rows. UNIQUE serves both the upsert's
        // conflict target and the per-pair lookup, so no extra index is needed.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_session_reminders (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    coaching_session_id UUID NOT NULL REFERENCES refactor_platform.coaching_sessions(id) ON DELETE CASCADE,
                    user_id UUID NOT NULL REFERENCES refactor_platform.users(id) ON DELETE CASCADE,
                    sent_for_start TIMESTAMP NOT NULL,
                    -- Regenerated on every claim so a caller can prove the claim it is
                    -- confirming or releasing is still its own. `sent_for_start` cannot
                    -- serve that purpose: a session moved away and back repeats a value.
                    claim_id UUID NOT NULL DEFAULT gen_random_uuid(),
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    CONSTRAINT uq_coaching_session_reminders_session_user UNIQUE (coaching_session_id, user_id)
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_reminders OWNER TO refactor",
            )
            .await?;

        // Suppress a deploy-time blast. Every session already inside the reminder
        // window (and every past one) is marked as though its reminder had gone out,
        // so enabling the job does not email every coachee with an imminent session
        // the moment the container starts. Sessions further out are left unstamped
        // and get a reminder on schedule.
        //
        // Coachees only: they are the sole recipient today. A recipient added later
        // needs its own suppression pass at that time, since a row written here would
        // claim a reminder was sent for a window that closed long before.
        let lead_hours = suppression_lead_hours();
        manager
            .get_connection()
            .execute_unprepared(&format!(
                "INSERT INTO refactor_platform.coaching_session_reminders
                     (coaching_session_id, user_id, sent_for_start)
                 SELECT cs.id, cr.coachee_id, cs.date
                 FROM refactor_platform.coaching_sessions cs
                 JOIN refactor_platform.coaching_relationships cr
                   ON cr.id = cs.coaching_relationship_id
                 WHERE cs.date <= NOW() AT TIME ZONE 'UTC' + INTERVAL '{lead_hours} hours'
                 ON CONFLICT (coaching_session_id, user_id) DO NOTHING"
            ))
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.coaching_session_reminders")
            .await?;

        Ok(())
    }
}
