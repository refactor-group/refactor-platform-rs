use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Create the token_purpose enum type.
        //    'setup' covers initial-password-set tokens issued at user invite.
        //    'password_reset' covers user-initiated reset tokens.
        //    Purpose separation prevents a leaked setup token from being
        //    redeemed at the reset endpoint and vice versa.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.token_purpose AS ENUM ('setup', 'password_reset')",
            )
            .await?;

        // 2. Transfer ownership to the 'refactor' role so future migrations
        //    running as that user can ALTER the type. See CLAUDE.md for context.
        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.token_purpose OWNER TO refactor")
            .await?;

        // 3. Add the column as nullable so the ALTER succeeds on tables with
        //    existing rows; backfill in step 4 before enforcing NOT NULL.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.magic_link_tokens \
                 ADD COLUMN purpose refactor_platform.token_purpose",
            )
            .await?;

        // 4. Backfill existing rows to 'setup'. Every token in production
        //    today was issued by the invite/welcome flow.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.magic_link_tokens SET purpose = 'setup' WHERE purpose IS NULL",
            )
            .await?;

        // 5. Enforce NOT NULL. No default — every insertion must declare
        //    its purpose explicitly, preventing accidental defaulting.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.magic_link_tokens \
                 ALTER COLUMN purpose SET NOT NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.magic_link_tokens DROP COLUMN IF EXISTS purpose",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.token_purpose")
            .await?;

        Ok(())
    }
}
