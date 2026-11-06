use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Local wall-clock working hours in the user's timezone, on a 30-minute step.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.users
                 ADD COLUMN working_hours_start TIME NOT NULL DEFAULT '08:00',
                 ADD COLUMN working_hours_end   TIME NOT NULL DEFAULT '17:30',
                 ADD CONSTRAINT users_working_hours_order CHECK (working_hours_start < working_hours_end),
                 ADD CONSTRAINT users_working_hours_start_step CHECK (
                   EXTRACT(SECOND FROM working_hours_start) = 0
                   AND EXTRACT(MINUTE FROM working_hours_start) IN (0, 30)),
                 ADD CONSTRAINT users_working_hours_end_step CHECK (
                   EXTRACT(SECOND FROM working_hours_end) = 0
                   AND EXTRACT(MINUTE FROM working_hours_end) IN (0, 30))",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.users
                 DROP CONSTRAINT IF EXISTS users_working_hours_end_step,
                 DROP CONSTRAINT IF EXISTS users_working_hours_start_step,
                 DROP CONSTRAINT IF EXISTS users_working_hours_order,
                 DROP COLUMN IF EXISTS working_hours_end,
                 DROP COLUMN IF EXISTS working_hours_start",
            )
            .await?;

        Ok(())
    }
}
