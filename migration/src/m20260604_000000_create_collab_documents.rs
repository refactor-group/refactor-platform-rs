use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Yjs document state, keyed by document name. Columns match the crate's
        // startup bootstrap so the bootstrap is a no-op once this has run.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.collab_documents (
                    name TEXT PRIMARY KEY,
                    state BYTEA NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE refactor_platform.collab_documents OWNER TO refactor")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.collab_documents")
            .await?;
        Ok(())
    }
}
