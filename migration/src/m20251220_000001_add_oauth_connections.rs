use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create the provider enum shared across oauth_connections and coaching_sessions.
        // Starting with 'google' only; add providers via ALTER TYPE ADD VALUE as needed.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.provider AS ENUM ('google')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.provider OWNER TO refactor")
            .await?;

        // Create oauth_connections table for storing per-user OAuth credentials.
        // Tokens are encrypted at the application layer via domain::encryption (AES-256-GCM).
        // Row existence = connected; deletion = disconnected. No soft-delete.
        let create_table_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.oauth_connections (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                user_id UUID NOT NULL REFERENCES refactor_platform.users(id) ON DELETE CASCADE,

                provider refactor_platform.provider NOT NULL,
                external_account_id VARCHAR(255),
                external_email VARCHAR(255),

                access_token TEXT NOT NULL,
                refresh_token TEXT,
                token_expires_at TIMESTAMPTZ,
                token_type VARCHAR(50) NOT NULL DEFAULT 'Bearer',
                scopes TEXT NOT NULL DEFAULT '',

                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

                UNIQUE(user_id, provider)
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.oauth_connections OWNER TO refactor",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_oauth_connections_user_provider
                 ON refactor_platform.oauth_connections(user_id, provider)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.oauth_connections")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.provider")
            .await?;

        Ok(())
    }
}
