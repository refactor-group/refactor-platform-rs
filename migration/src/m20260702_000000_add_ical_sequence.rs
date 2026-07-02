use sea_orm_migration::prelude::*;

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

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
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
