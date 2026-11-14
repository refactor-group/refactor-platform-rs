use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // When this participant was last told this session's start: at booking, and again
        // on every reschedule email they receive. Replaces coaching_sessions.notice_given_at
        // as the reminder sweep's measure of notice once the sweep reads it, so one
        // participant's email won't count as notice to the other. UNIQUE serves the
        // upsert's conflict target and the lookup.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_session_notices (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    coaching_session_id UUID NOT NULL REFERENCES refactor_platform.coaching_sessions(id) ON DELETE CASCADE,
                    user_id UUID NOT NULL REFERENCES refactor_platform.users(id) ON DELETE CASCADE,
                    notice_given_at TIMESTAMPTZ NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    CONSTRAINT uq_coaching_session_notices_session_user UNIQUE (coaching_session_id, user_id)
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_notices OWNER TO refactor",
            )
            .await?;

        // Both participants were told whenever the session as a whole was. DO NOTHING
        // covers a coach coaching themselves, which only data older than the create check
        // could hold.
        manager
            .get_connection()
            .execute_unprepared(
                "INSERT INTO refactor_platform.coaching_session_notices
                     (coaching_session_id, user_id, notice_given_at)
                 SELECT cs.id, participant.user_id, cs.notice_given_at
                 FROM refactor_platform.coaching_sessions cs
                 JOIN refactor_platform.coaching_relationships cr
                   ON cr.id = cs.coaching_relationship_id
                 CROSS JOIN LATERAL (VALUES (cr.coach_id), (cr.coachee_id)) AS participant(user_id)
                 ON CONFLICT (coaching_session_id, user_id) DO NOTHING",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.coaching_session_notices")
            .await?;

        Ok(())
    }
}
