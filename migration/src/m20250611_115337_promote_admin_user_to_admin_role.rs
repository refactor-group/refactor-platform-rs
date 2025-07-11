use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Use raw SQL to avoid dependency on entity models
        let sql = r#"
            UPDATE refactor_platform.users 
            SET role = 'admin' 
            WHERE email = 'admin@refactorcoach.com'
        "#;

        db.execute(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            vec![],
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Use raw SQL for rollback as well
        let sql = r#"
            UPDATE refactor_platform.users 
            SET role = 'user' 
            WHERE email = 'admin@refactorcoach.com'
        "#;

        db.execute(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            vec![],
        ))
        .await?;

        Ok(())
    }
}
