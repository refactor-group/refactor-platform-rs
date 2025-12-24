use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Rename existing ai_privacy_level column to coach_ai_privacy_level
        // This makes it explicit that this is the coach's consent setting
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_relationships
                 RENAME COLUMN ai_privacy_level TO coach_ai_privacy_level",
            )
            .await?;

        // Add coachee_ai_privacy_level column for the coachee's consent
        // Default is 'full' so existing relationships don't break (opt-in by default)
        // Both coach AND coachee must consent for AI features to be available
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_relationships
                 ADD COLUMN coachee_ai_privacy_level refactor_platform.ai_privacy_level NOT NULL DEFAULT 'full'",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove the coachee column
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_relationships
                 DROP COLUMN IF EXISTS coachee_ai_privacy_level",
            )
            .await?;

        // Rename coach_ai_privacy_level back to ai_privacy_level
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_relationships
                 RENAME COLUMN coach_ai_privacy_level TO ai_privacy_level",
            )
            .await?;

        Ok(())
    }
}
