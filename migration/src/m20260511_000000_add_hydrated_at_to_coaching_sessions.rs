use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Adds the hydration sentinel. NULL means "deferred side-effects
        // (Tiptap doc, meeting URL, goal-link) have not yet run for this row";
        // non-NULL means they ran to a definitive decision and won't be re-attempted.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN hydrated_at TIMESTAMPTZ",
            )
            .await?;

        // Backfill: every pre-existing row was created by the eager path, so its
        // side-effects already ran. Mark them hydrated using created_at so the
        // read-path hydration check skips them.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_sessions
                 SET hydrated_at = created_at
                 WHERE hydrated_at IS NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN IF EXISTS hydrated_at",
            )
            .await?;

        Ok(())
    }
}
