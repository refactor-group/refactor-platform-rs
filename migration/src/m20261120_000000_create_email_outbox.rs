use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // One row per email owed to one recipient, written in the transaction it announces.
        // `not_before` and `lease_until` are naive UTC so app-side times compare in any engine.
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS refactor_platform.email_outbox (
                id UUID PRIMARY KEY,
                kind TEXT NOT NULL,
                subject_key TEXT NOT NULL,
                recipient_user_id UUID NOT NULL
                    REFERENCES refactor_platform.users(id) ON UPDATE CASCADE ON DELETE CASCADE,
                position SMALLINT NOT NULL,
                request JSONB NULL,
                state TEXT NOT NULL DEFAULT 'pending',
                attempts SMALLINT NOT NULL DEFAULT 0,
                not_before TIMESTAMP NOT NULL,
                claim_id UUID NULL,
                lease_until TIMESTAMP NULL,
                last_error TEXT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                finished_at TIMESTAMPTZ NULL,
                CONSTRAINT email_outbox_state
                    CHECK (state IN ('pending', 'sent', 'failed')),
                CONSTRAINT email_outbox_finished_when_not_pending
                    CHECK ((state = 'pending') = (finished_at IS NULL)),
                CONSTRAINT email_outbox_pending_has_request
                    CHECK (state <> 'pending' OR request IS NOT NULL)
            )",
        )
        .await?;

        // Serves the claim's scan of due rows.
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS email_outbox_pending_not_before_idx
                ON refactor_platform.email_outbox (not_before)
                WHERE state = 'pending'",
        )
        .await?;

        // Serves the claim's check for an older pending row in the same conversation.
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS email_outbox_pending_order_idx
                ON refactor_platform.email_outbox
                    (subject_key, recipient_user_id, created_at, position)
                WHERE state = 'pending'",
        )
        .await?;

        // Serves the purge of finished rows.
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS email_outbox_finished_at_idx
                ON refactor_platform.email_outbox (finished_at)",
        )
        .await?;

        // Serves a user delete's cascade.
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS email_outbox_recipient_user_idx
                ON refactor_platform.email_outbox (recipient_user_id)",
        )
        .await?;

        db.execute_unprepared("ALTER TABLE refactor_platform.email_outbox OWNER TO refactor")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.email_outbox")
            .await?;
        Ok(())
    }
}
