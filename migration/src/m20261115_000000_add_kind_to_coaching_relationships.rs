use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The kinds a coaching relationship can be.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.coaching_relationship_kind \
                 AS ENUM ('coaching', 'colleague')",
            )
            .await?;

        // Owned by 'refactor' so later migrations can ALTER it. See CLAUDE.md.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TYPE refactor_platform.coaching_relationship_kind OWNER TO refactor",
            )
            .await?;

        // Existing rows take the default.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_relationships \
                 ADD COLUMN kind refactor_platform.coaching_relationship_kind \
                 NOT NULL DEFAULT 'coaching'",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_relationships DROP COLUMN IF EXISTS kind",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.coaching_relationship_kind")
            .await?;

        Ok(())
    }
}
