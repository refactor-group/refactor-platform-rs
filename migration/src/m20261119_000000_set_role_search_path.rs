use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A role default reaches every new server connection, including ones a
        // transaction-mode pooler opens, where a per-connection SET would not survive.
        manager
            .get_connection()
            .execute_unprepared(
                "DO $$ BEGIN
                     EXECUTE format(
                         'ALTER ROLE %I IN DATABASE %I SET search_path = refactor_platform',
                         current_user, current_database()
                     );
                 END $$",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DO $$ BEGIN
                     EXECUTE format(
                         'ALTER ROLE %I IN DATABASE %I RESET search_path',
                         current_user, current_database()
                     );
                 END $$",
            )
            .await?;

        Ok(())
    }
}
