use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Add super_admin variant to the existing role enum
        // PostgreSQL's ALTER TYPE ... ADD VALUE cannot be run inside a transaction block
        // but SeaORM wraps migrations in transactions. We use IF NOT EXISTS for safety.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TYPE refactor_platform.role ADD VALUE IF NOT EXISTS 'super_admin'",
            )
            .await?;

        // 2. Create the user_roles table
        // Using raw SQL to handle PostgreSQL schema qualification properly
        let create_table_sql = "CREATE TABLE IF NOT EXISTS refactor_platform.user_roles (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            role refactor_platform.role NOT NULL,
            organization_id UUID,
            user_id UUID NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            CONSTRAINT fk_user_roles_organization 
                FOREIGN KEY (organization_id) 
                REFERENCES refactor_platform.organizations(id) 
                ON DELETE CASCADE 
                ON UPDATE CASCADE,
            CONSTRAINT fk_user_roles_user 
                FOREIGN KEY (user_id) 
                REFERENCES refactor_platform.users(id) 
                ON DELETE CASCADE 
                ON UPDATE CASCADE
        )";

        manager
            .get_connection()
            .execute_unprepared(create_table_sql)
            .await?;

        // 3. Create unique index to prevent duplicate role assignments
        let create_index_sql = "CREATE UNIQUE INDEX IF NOT EXISTS user_roles_user_org_role_unique 
            ON refactor_platform.user_roles(user_id, organization_id, role)";

        manager
            .get_connection()
            .execute_unprepared(create_index_sql)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the user_roles table (this will also drop the indexes and foreign keys)
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.user_roles")
            .await?;

        // Note: We cannot remove the 'super_admin' value from the enum in PostgreSQL
        // once it has been added. This is a PostgreSQL limitation.
        // If you need to truly remove it, you would need to:
        // 1. Create a new enum type without 'super_admin'
        // 2. Update all columns using the old type to use the new type
        // 3. Drop the old enum type
        // This is complex and risky, so we'll leave the enum value in place.

        Ok(())
    }
}
