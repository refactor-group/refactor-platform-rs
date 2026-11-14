use sea_orm::Statement;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Session notes live in collab documents; refuse rather than discard a row.
        let count: i64 = conn
            .query_one_raw(Statement::from_string(
                conn.get_database_backend(),
                "SELECT COUNT(*) AS note_count FROM refactor_platform.notes",
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("Counting notes returned no row".to_string()))?
            .try_get("", "note_count")?;

        if count > 0 {
            return Err(DbErr::Custom(format!(
                "refactor_platform.notes holds {count} rows; migrate or remove them before dropping the table"
            )));
        }

        conn.execute_unprepared("DROP TABLE IF EXISTS refactor_platform.notes")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS refactor_platform.notes (
                id UUID UNIQUE PRIMARY KEY NOT NULL DEFAULT gen_random_uuid(),
                coaching_session_id UUID NOT NULL
                    REFERENCES refactor_platform.coaching_sessions (id),
                body VARCHAR,
                user_id UUID NOT NULL REFERENCES refactor_platform.users (id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )",
        )
        .await?;

        conn.execute_unprepared("ALTER TABLE refactor_platform.notes OWNER TO refactor")
            .await?;

        Ok(())
    }
}
