use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Backs the case-insensitive email lookup behind `GET /users?email=`.
///
/// `users_email_key` indexes the raw column, so `LOWER(email) = LOWER($1)`
/// cannot use it and falls back to a sequential scan. The route is reachable by
/// any organization admin, so this keeps it from degrading as `users` grows.
const CREATE_INDEX_SQL: &str = r#"
    CREATE INDEX IF NOT EXISTS users_lower_email_idx
    ON refactor_platform.users (LOWER(email))
"#;

const DROP_INDEX_SQL: &str = r#"
    DROP INDEX IF EXISTS refactor_platform.users_lower_email_idx
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(CREATE_INDEX_SQL)
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(DROP_INDEX_SQL)
            .await?;
        Ok(())
    }
}
