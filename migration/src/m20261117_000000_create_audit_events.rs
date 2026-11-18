use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Who changed what in an organization. Each row's `hash` covers its fields and the
        // previous row's hash in the same organization, so an edited or removed row breaks the
        // chain. `prev_hash` is null only on an organization's first event. No foreign keys:
        // events outlive the organization, users and records they describe.
        db.execute_unprepared(
            "CREATE TABLE refactor_platform.audit_events (
                id              UUID PRIMARY KEY,
                organization_id UUID NOT NULL,
                actor_user_id   UUID,
                action          TEXT NOT NULL,
                subject_type    TEXT NOT NULL,
                subject_id      UUID NOT NULL,
                details         JSONB NOT NULL DEFAULT '{}',
                occurred_at     TIMESTAMPTZ NOT NULL,
                prev_hash       TEXT,
                hash            TEXT NOT NULL
            )",
        )
        .await?;

        db.execute_unprepared("ALTER TABLE refactor_platform.audit_events OWNER TO refactor")
            .await?;

        // An organization's events in chain order.
        db.execute_unprepared(
            "CREATE INDEX audit_events_organization_occurred_at_id \
             ON refactor_platform.audit_events (organization_id, occurred_at, id)",
        )
        .await?;

        // The table's owner is the app role, so revoked grants would not hold; a trigger does.
        db.execute_unprepared(
            "CREATE FUNCTION refactor_platform.audit_events_refuse_change() \
             RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN \
                 RAISE EXCEPTION 'audit_events is append-only: % refused', TG_OP; \
             END; \
             $$",
        )
        .await?;

        // Owned by 'refactor' so later migrations can replace it. See CLAUDE.md.
        db.execute_unprepared(
            "ALTER FUNCTION refactor_platform.audit_events_refuse_change() OWNER TO refactor",
        )
        .await?;

        db.execute_unprepared(
            "CREATE TRIGGER audit_events_append_only \
             BEFORE UPDATE OR DELETE ON refactor_platform.audit_events \
             FOR EACH ROW EXECUTE FUNCTION refactor_platform.audit_events_refuse_change()",
        )
        .await?;

        // Row triggers do not fire on TRUNCATE, which would empty every chain at once.
        db.execute_unprepared(
            "CREATE TRIGGER audit_events_no_truncate \
             BEFORE TRUNCATE ON refactor_platform.audit_events \
             FOR EACH STATEMENT EXECUTE FUNCTION refactor_platform.audit_events_refuse_change()",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Dropping the table drops its index and triggers.
        db.execute_unprepared("DROP TABLE IF EXISTS refactor_platform.audit_events")
            .await?;

        db.execute_unprepared(
            "DROP FUNCTION IF EXISTS refactor_platform.audit_events_refuse_change()",
        )
        .await?;

        Ok(())
    }
}
