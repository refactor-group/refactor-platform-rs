use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.provider ADD VALUE 'zoom'")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. DELETE coaching_sessiona and oauth_connections of the 'zoom' provider
        manager
            .get_connection()
            .execute_unprepared(
                "DELETE FROM refactor_platform.oauth_connections WHERE provider = 'zoom'",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "DELETE FROM refactor_platform.coaching_sessions WHERE provider = 'zoom'",
            )
            .await?;

        // 2. Rename the old enum (so we can create a new one with the original name)
        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.provider RENAME TO provider_old")
            .await?;

        // 3. Create a new enum with the only desired value
        manager
            .get_connection()
            .execute_unprepared("CREATE TYPE refactor_platform.provider AS ENUM ('google')")
            .await?;

        // 4. Update the oauth_connections and coaching_sessions to use the new enum
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.oauth_connections
                ALTER COLUMN provider TYPE refactor_platform.provider
                USING provider::text::refactor_platform.provider",
            )
            .await?;
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                ALTER COLUMN provider TYPE refactor_platform.provider
                USING provider::text::refactor_platform.provider",
            )
            .await?;

        // 5. Drop the old enum
        manager
            .get_connection()
            .execute_unprepared("DROP TYPE refactor_platform.provider_old")
            .await?;

        Ok(())
    }
}
