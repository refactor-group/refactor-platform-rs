use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS organizations_name_key \
                 ON refactor_platform.organizations(name)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS organizations_slug_key \
                 ON refactor_platform.organizations(slug)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS refactor_platform.organizations_name_key")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS refactor_platform.organizations_slug_key")
            .await?;

        Ok(())
    }
}
