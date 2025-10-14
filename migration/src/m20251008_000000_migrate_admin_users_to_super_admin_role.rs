use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // For each user with role = 'admin', create a user_roles record with:
        // - role = 'super_admin'
        // - organization_id = NULL (global/super admin has no org association)
        // - user_id = the admin user's id
        //
        // We use ON CONFLICT DO NOTHING to handle cases where this migration
        // might be run multiple times (idempotent operation)
        //
        // Note: We explicitly cast 'super_admin' to the enum type to handle
        // PostgreSQL's transaction safety restrictions when using newly added enum values
        let insert_super_admin_roles_sql = r#"
            INSERT INTO refactor_platform.user_roles (user_id, role, organization_id)
            SELECT id, 'super_admin'::refactor_platform.role, NULL
            FROM refactor_platform.users
            WHERE role = 'admin'
            ON CONFLICT DO NOTHING
        "#;

        manager
            .get_connection()
            .execute_unprepared(insert_super_admin_roles_sql)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove the super_admin user_roles records that were created for admin users
        // This only removes super_admin roles with NULL organization_id for users
        // who currently have role = 'admin'
        let delete_super_admin_roles_sql = r#"
            DELETE FROM refactor_platform.user_roles
            WHERE role = 'super_admin'::refactor_platform.role
              AND organization_id IS NULL
              AND user_id IN (
                  SELECT id
                  FROM refactor_platform.users
                  WHERE role = 'admin'
              )
        "#;

        manager
            .get_connection()
            .execute_unprepared(delete_super_admin_roles_sql)
            .await?;

        Ok(())
    }
}
