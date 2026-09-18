use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Only the column goes. The refactor_platform.role type still backs user_roles.role.
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE refactor_platform.users DROP COLUMN role")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Hazard: m20250610_104530_add_role_to_users down() drops the role type, which
        // user_roles now depends on. Left as is (historical, rollback only).
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.users \
                 ADD COLUMN role refactor_platform.role NOT NULL DEFAULT 'user'",
            )
            .await?;

        Ok(())
    }
}
