use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add stated_by_user_id - who said this item (from speaker diarization)
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.ai_suggested_items
                 ADD COLUMN stated_by_user_id UUID REFERENCES refactor_platform.users(id)",
            )
            .await?;

        // Add assigned_to_user_id - who should complete this item (from LeMUR analysis)
        // NULL for agreements since they are mutual commitments with no single assignee
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.ai_suggested_items
                 ADD COLUMN assigned_to_user_id UUID REFERENCES refactor_platform.users(id)",
            )
            .await?;

        // Add source_segment_id - link to the transcript segment for provenance
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.ai_suggested_items
                 ADD COLUMN source_segment_id UUID REFERENCES refactor_platform.transcript_segments(id)",
            )
            .await?;

        // Create indexes for efficient querying by assignee
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX idx_ai_suggested_items_stated_by
                 ON refactor_platform.ai_suggested_items(stated_by_user_id)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX idx_ai_suggested_items_assigned_to
                 ON refactor_platform.ai_suggested_items(assigned_to_user_id)",
            )
            .await?;

        // Add auto_approve_ai_suggestions setting to user_integrations
        // Default is false - coaches must review suggestions before they become actions/agreements
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.user_integrations
                 ADD COLUMN auto_approve_ai_suggestions BOOLEAN NOT NULL DEFAULT false",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove auto_approve_ai_suggestions from user_integrations
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.user_integrations
                 DROP COLUMN IF EXISTS auto_approve_ai_suggestions",
            )
            .await?;

        // Drop indexes
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX IF EXISTS refactor_platform.idx_ai_suggested_items_assigned_to",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX IF EXISTS refactor_platform.idx_ai_suggested_items_stated_by",
            )
            .await?;

        // Remove columns from ai_suggested_items
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.ai_suggested_items
                 DROP COLUMN IF EXISTS source_segment_id",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.ai_suggested_items
                 DROP COLUMN IF EXISTS assigned_to_user_id",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.ai_suggested_items
                 DROP COLUMN IF EXISTS stated_by_user_id",
            )
            .await?;

        Ok(())
    }
}
