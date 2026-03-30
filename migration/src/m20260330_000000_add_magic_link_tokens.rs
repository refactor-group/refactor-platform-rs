use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Make users.password nullable so users can be created without a password
        // (they will set it via the magic link flow).
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.users ALTER COLUMN password DROP NOT NULL",
            )
            .await?;

        // Create magic_link_tokens table for invite/setup tokens.
        // Only the SHA-256 hash of each token is stored; the raw token
        // appears only in the email URL.
        let create_table_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.magic_link_tokens (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                user_id UUID NOT NULL REFERENCES refactor_platform.users(id) ON DELETE CASCADE,
                token_hash VARCHAR(64) NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE(token_hash)
            )
        "#;

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE refactor_platform.magic_link_tokens OWNER TO refactor")
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_magic_link_tokens_user_id
                 ON refactor_platform.magic_link_tokens(user_id)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.magic_link_tokens")
            .await?;

        // Re-add NOT NULL constraint to users.password.
        // Any rows with NULL password will need a placeholder first.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.users SET password = '' WHERE password IS NULL",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.users ALTER COLUMN password SET NOT NULL",
            )
            .await?;

        Ok(())
    }
}
