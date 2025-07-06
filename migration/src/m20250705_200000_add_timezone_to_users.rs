use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add the 'timezone' column to 'users' table, default 'UTC', not null
        manager
            .alter_table(
                Table::alter()
                    .table((Alias::new("refactor_platform"), Alias::new("users")))
                    .add_column(
                        ColumnDef::new(Alias::new("timezone"))
                            .string_len(50)
                            .not_null()
                            .default("UTC"),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the 'timezone' column from 'users' table
        manager
            .alter_table(
                Table::alter()
                    .table((Alias::new("refactor_platform"), Alias::new("users")))
                    .drop_column(Alias::new("timezone"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
