use sea_orm_migration::prelude::*;

/// Adds the RFC 5545 protocol columns: a persisted `SEQUENCE` counter on both
/// coaching sessions and series, plus `ical_recurrence_id` on sessions, which
/// addresses one occurrence within a recurring event.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Persisted RFC 5545 SEQUENCE counter, backfilled to 0 on existing rows.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN ical_sequence INTEGER NOT NULL DEFAULT 0",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_series
                 ADD COLUMN ical_sequence INTEGER NOT NULL DEFAULT 0",
            )
            .await?;

        // RFC 5545 RECURRENCE-ID: the occurrence's original start, naive UTC like
        // `date`. NULL for standalone sessions; never rewritten once materialized.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN ical_recurrence_id TIMESTAMP NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS ical_recurrence_id",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_session_series
                 DROP COLUMN IF EXISTS ical_sequence",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS ical_sequence",
            )
            .await?;

        Ok(())
    }
}
