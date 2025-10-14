use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // For each user with role = 'user', find their associated organizations
        // and create a user_roles record for each organization with:
        // - role = 'user'
        // - organization_id = the organization's id from organizations_users
        // - user_id = the user's id
        //
        // We use ON CONFLICT DO NOTHING to handle cases where this migration
        // might be run multiple times (idempotent operation)
        //
        // Note: We explicitly cast 'user' to the enum type to handle
        // PostgreSQL's transaction safety restrictions when using enum values
        let insert_user_roles_sql = r#"
            INSERT INTO refactor_platform.user_roles (user_id, role, organization_id)
            SELECT
                u.id,
                'user'::refactor_platform.role,
                ou.organization_id
            FROM refactor_platform.users u
            INNER JOIN refactor_platform.organizations_users ou ON u.id = ou.user_id
            WHERE u.role = 'user'
            ON CONFLICT DO NOTHING
        "#;

        manager
            .get_connection()
            .execute_unprepared(insert_user_roles_sql)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove the user role user_roles records that were created for regular users
        // This only removes 'user' roles for users who currently have role = 'user'
        // and only for organizations they're actually associated with
        let delete_user_roles_sql = r#"
            DELETE FROM refactor_platform.user_roles
            WHERE role = 'user'::refactor_platform.role
              AND user_id IN (
                  SELECT id
                  FROM refactor_platform.users
                  WHERE role = 'user'
              )
              AND organization_id IN (
                  SELECT organization_id
                  FROM refactor_platform.organizations_users
                  WHERE user_id = refactor_platform.user_roles.user_id
              )
        "#;

        manager
            .get_connection()
            .execute_unprepared(delete_user_roles_sql)
            .await?;

        Ok(())
    }
}
