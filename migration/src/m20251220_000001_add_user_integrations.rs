use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create the user_integrations table for storing encrypted API credentials
        // This table allows coaches to configure their Google OAuth, Recall.ai, and AssemblyAI credentials
        let create_table_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.user_integrations (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                user_id UUID NOT NULL REFERENCES refactor_platform.users(id) ON DELETE CASCADE,

                -- Google OAuth (encrypted)
                google_access_token TEXT,
                google_refresh_token TEXT,
                google_token_expiry TIMESTAMPTZ,
                google_email VARCHAR(255),

                -- Recall.ai (encrypted)
                recall_ai_api_key TEXT,
                recall_ai_region VARCHAR(50) DEFAULT 'us-west-2',
                recall_ai_verified_at TIMESTAMPTZ,

                -- AssemblyAI (encrypted)
                assembly_ai_api_key TEXT,
                assembly_ai_verified_at TIMESTAMPTZ,

                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

                CONSTRAINT user_integrations_user_id_unique UNIQUE(user_id)
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        // Set ownership to refactor user for proper permissions
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE refactor_platform.user_integrations OWNER TO refactor")
            .await?;

        // Create index for faster lookups by user_id
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_user_integrations_user_id
                 ON refactor_platform.user_integrations(user_id)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.user_integrations")
            .await?;

        Ok(())
    }
}
