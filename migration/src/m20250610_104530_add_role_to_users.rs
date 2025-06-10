use sea_orm::{EnumIter, Iterable};
use sea_orm_migration::prelude::extension::postgres::Type;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
struct Role;

#[derive(DeriveIden, EnumIter)]
enum RoleVariants {
    User,
    Admin,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_type(
                Type::create()
                    .as_enum(Role)
                    .values(RoleVariants::iter())
                    .to_owned(),
            )
            .await?;

        // 2. Add the 'role' column to 'users' table, default 'user', not null
        manager
            .alter_table(
                Table::alter()
                    .table("users")
                    .add_column(
                        ColumnDef::new("role")
                            .custom("refactor_platform.role")
                            .not_null()
                            .default("user"),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Drop the 'role' column from 'users' table
        manager
            .alter_table(Table::alter().table("users").drop_column("role").to_owned())
            .await?;

        // 2. Drop the ENUM type 'role'
        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.role")
            .await?;

        Ok(())
    }
}

// Removed DeriveIden enums. Using Alias::new for all identifiers.
