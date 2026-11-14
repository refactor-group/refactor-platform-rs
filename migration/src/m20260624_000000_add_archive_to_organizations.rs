use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Archive audit markers; null for live orgs. archived_by set null on user delete.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.organizations \
                 ADD COLUMN archived_at TIMESTAMPTZ, \
                 ADD COLUMN archived_by UUID, \
                 ADD CONSTRAINT fk_organizations_archived_by \
                   FOREIGN KEY (archived_by) \
                   REFERENCES refactor_platform.users(id) \
                   ON DELETE SET NULL \
                   ON UPDATE CASCADE",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.organizations \
                 DROP CONSTRAINT IF EXISTS fk_organizations_archived_by, \
                 DROP COLUMN archived_by, \
                 DROP COLUMN archived_at",
            )
            .await?;

        Ok(())
    }
}
