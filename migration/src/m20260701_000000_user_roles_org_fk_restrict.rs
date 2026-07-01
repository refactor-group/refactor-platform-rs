use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Flip user_roles.organization_id from ON DELETE CASCADE to RESTRICT so
        // the database enforces block-until-empty. The app deletes an org only
        // after counting members, but at READ COMMITTED a member-add can commit
        // between that count and the DELETE; under CASCADE the new role grant is
        // then silently dropped. RESTRICT rejects the delete instead.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.user_roles \
                   DROP CONSTRAINT IF EXISTS fk_user_roles_organization; \
                 ALTER TABLE refactor_platform.user_roles \
                   ADD CONSTRAINT fk_user_roles_organization \
                   FOREIGN KEY (organization_id) \
                   REFERENCES refactor_platform.organizations(id) \
                   ON DELETE RESTRICT \
                   ON UPDATE CASCADE",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.user_roles \
                   DROP CONSTRAINT IF EXISTS fk_user_roles_organization; \
                 ALTER TABLE refactor_platform.user_roles \
                   ADD CONSTRAINT fk_user_roles_organization \
                   FOREIGN KEY (organization_id) \
                   REFERENCES refactor_platform.organizations(id) \
                   ON DELETE CASCADE \
                   ON UPDATE CASCADE",
            )
            .await?;

        Ok(())
    }
}
