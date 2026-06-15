use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Optional human-authored session title. Nullable, no default.
        // Bounded to 500 chars to match the sibling meeting_url column and
        // guard against runaway input at the DB layer.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions ADD COLUMN title VARCHAR(500)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE refactor_platform.coaching_sessions DROP COLUMN title")
            .await?;

        Ok(())
    }
}
