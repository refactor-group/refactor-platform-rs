use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One row per invite owed to one participant: a session's own invite or a series'.
        // Each UNIQUE is a queue write's conflict target, so one invite is owed at most once.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.coaching_session_pending_invites (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    user_id UUID NOT NULL
                        REFERENCES refactor_platform.users(id) ON UPDATE CASCADE ON DELETE CASCADE,
                    coaching_session_id UUID NULL
                        REFERENCES refactor_platform.coaching_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE,
                    coaching_session_series_id UUID NULL
                        REFERENCES refactor_platform.coaching_session_series(id) ON UPDATE CASCADE ON DELETE CASCADE,
                    attempts SMALLINT NOT NULL DEFAULT 0,
                    claim_id UUID NULL,
                    claimed_at TIMESTAMP NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    CONSTRAINT coaching_session_pending_invites_one_target
                        CHECK ((coaching_session_id IS NULL) <> (coaching_session_series_id IS NULL)),
                    CONSTRAINT uq_coaching_session_pending_invites_session_user
                        UNIQUE (coaching_session_id, user_id),
                    CONSTRAINT uq_coaching_session_pending_invites_series_user
                        UNIQUE (coaching_session_series_id, user_id)
                )",
            )
            .await?;

        // The unique keys lead with the target, so a user delete's cascade needs its own.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS coaching_session_pending_invites_user_idx
                    ON refactor_platform.coaching_session_pending_invites (user_id)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_pending_invites OWNER TO refactor",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP TABLE IF EXISTS refactor_platform.coaching_session_pending_invites",
            )
            .await?;
        Ok(())
    }
}
