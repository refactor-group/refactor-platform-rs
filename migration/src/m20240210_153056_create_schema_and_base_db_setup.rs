use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create the platform's schema
        manager
            .get_connection()
            .execute_unprepared("CREATE SCHEMA IF NOT EXISTS refactor_platform;")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("SET search_path TO refactor_platform, public;")
            .await?;

        // Create the base DB user that will execute all platform queries
        manager
            .get_connection()
            .execute_unprepared(r#"
                DO $$ BEGIN
                    GRANT ALL PRIVILEGES ON DATABASE refactor TO refactor;
                    GRANT ALL ON SCHEMA refactor_platform TO refactor;

                    ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform GRANT ALL ON TABLES TO refactor;
                    ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform GRANT ALL ON SEQUENCES TO refactor;
                    ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform GRANT ALL ON FUNCTIONS TO refactor;
                END $$;
            "#)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Revoke default privileges first
        manager
            .get_connection()
            .execute_unprepared(r#"
                DO $$ BEGIN
                    ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform REVOKE ALL ON FUNCTIONS FROM refactor;
                    ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform REVOKE ALL ON SEQUENCES FROM refactor;
                    ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform REVOKE ALL ON TABLES FROM refactor;
                    REVOKE ALL ON SCHEMA refactor_platform FROM refactor;
                    REVOKE ALL PRIVILEGES ON DATABASE refactor FROM refactor;
                END $$;
            "#)
            .await?;

        // Drop the schema (CASCADE will remove all objects in it)
        manager
            .get_connection()
            .execute_unprepared("DROP SCHEMA IF EXISTS refactor_platform CASCADE;")
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Post {
    Table,
    Id,
    Title,
    Text,
}
