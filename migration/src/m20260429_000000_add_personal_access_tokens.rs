use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create the pat_status enum (separate from the existing `status` enum)
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.pat_status AS ENUM ('active', 'inactive')",
            )
            .await?;

        // Create personal_access_tokens table
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.personal_access_tokens (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                user_id UUID NOT NULL REFERENCES refactor_platform.users(id) ON DELETE CASCADE,
                token_hash VARCHAR(64) NOT NULL UNIQUE,
                status refactor_platform.pat_status NOT NULL DEFAULT 'active',
                last_used_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.personal_access_tokens OWNER TO refactor",
            )
            .await?;

        // Partial unique index: only one active PAT per user
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX idx_personal_access_tokens_one_active_per_user
                 ON refactor_platform.personal_access_tokens(user_id)
                 WHERE status = 'active'",
            )
            .await?;

        // Index on token_hash for fast lookups during auth
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_personal_access_tokens_token_hash
                 ON refactor_platform.personal_access_tokens(token_hash)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.personal_access_tokens")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.pat_status")
            .await?;

        Ok(())
    }
}
