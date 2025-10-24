use sea_orm::Statement;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Validate all organizations_users have matching user_roles
        // Exception: super_admin users are allowed to have organizations_users records
        // without matching organization-scoped user_roles, since super_admins have
        // global access (organization_id = NULL in user_roles)
        let conn = manager.get_connection();
        let backend = conn.get_database_backend();

        let validation_sql = r#"
            SELECT COUNT(*) as orphan_count
            FROM refactor_platform.organizations_users ou
            LEFT JOIN refactor_platform.user_roles ur
              ON ou.user_id = ur.user_id
              AND ou.organization_id = ur.organization_id
            WHERE ur.id IS NULL
              AND NOT EXISTS (
                  SELECT 1
                  FROM refactor_platform.user_roles super_admin_ur
                  WHERE super_admin_ur.user_id = ou.user_id
                    AND super_admin_ur.role = 'super_admin'
                    AND super_admin_ur.organization_id IS NULL
              )
        "#;

        let result = conn
            .query_one(Statement::from_string(backend, validation_sql))
            .await?
            .ok_or_else(|| DbErr::Custom("Validation query failed".to_string()))?;

        let count: i64 = result
            .try_get("", "orphan_count")
            .map_err(|e| DbErr::Custom(format!("Failed to parse count: {}", e)))?;

        if count > 0 {
            return Err(DbErr::Custom(format!(
                "Found {} organizations_users records without matching user_roles. \
                Each organizations_users record must have a corresponding user_roles record \
                with the same user_id and organization_id before this table can be removed. \
                (Super admin users are excluded from this check as they have global access.)",
                count
            )));
        }

        // Create index on user_roles.organization_id for optimized queries
        // This is needed because queries previously using organizations_users will now use user_roles
        let create_index_sql = r#"
            CREATE INDEX IF NOT EXISTS idx_user_roles_organization_id
            ON refactor_platform.user_roles(organization_id)
        "#;
        conn.execute_unprepared(create_index_sql).await?;

        // Drop the organizations_users table
        let drop_table_sql = r#"
            DROP TABLE IF EXISTS refactor_platform.organizations_users
        "#;
        conn.execute_unprepared(drop_table_sql).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Recreate organizations_users table
        let create_table_sql = r#"
            CREATE TABLE IF NOT EXISTS refactor_platform.organizations_users (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                organization_id UUID NOT NULL,
                user_id UUID NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT fk_organizations_users_organization
                    FOREIGN KEY (organization_id)
                    REFERENCES refactor_platform.organizations(id)
                    ON DELETE NO ACTION
                    ON UPDATE NO ACTION,
                CONSTRAINT fk_organizations_users_user
                    FOREIGN KEY (user_id)
                    REFERENCES refactor_platform.users(id)
                    ON DELETE NO ACTION
                    ON UPDATE NO ACTION
            )
        "#;
        conn.execute_unprepared(create_table_sql).await?;

        // Add unique constraint to prevent duplicate entries on rollback
        let create_unique_index_sql = r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_organizations_users_unique
            ON refactor_platform.organizations_users(user_id, organization_id)
        "#;
        conn.execute_unprepared(create_unique_index_sql).await?;

        // Repopulate from user_roles where organization_id IS NOT NULL
        let repopulate_sql = r#"
            INSERT INTO refactor_platform.organizations_users (user_id, organization_id, created_at, updated_at)
            SELECT user_id, organization_id, created_at, updated_at
            FROM refactor_platform.user_roles
            WHERE organization_id IS NOT NULL
            ON CONFLICT (user_id, organization_id) DO NOTHING
        "#;
        conn.execute_unprepared(repopulate_sql).await?;

        // Drop the index that was added in up()
        let drop_index_sql = r#"
            DROP INDEX IF EXISTS refactor_platform.idx_user_roles_organization_id
        "#;
        conn.execute_unprepared(drop_index_sql).await?;

        Ok(())
    }
}
