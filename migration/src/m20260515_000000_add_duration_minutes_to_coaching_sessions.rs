use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add the per-session duration in minutes. SMALLINT (i16 in PG) saves 2
        // bytes per row vs INTEGER and comfortably accommodates the 1..=480
        // application-layer range enforced by the `Duration` newtype.
        //
        // DEFAULT 60 backfills existing rows in a single column-add — no separate
        // UPDATE pass needed.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN duration_minutes SMALLINT NOT NULL DEFAULT 60",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS duration_minutes",
            )
            .await?;

        Ok(())
    }
}
