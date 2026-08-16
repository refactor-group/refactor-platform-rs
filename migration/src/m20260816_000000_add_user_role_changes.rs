use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Append-only history of organization role grants, changes and removals.
        // `user_roles` is a state table (one row per user and organization), so it
        // cannot answer "was this person ever an admin?" after a demote.
        //
        // Nullability carries the semantics: no previous role is a first grant, no
        // new role is a removal, both set is a change. The CHECK rejects the
        // meaningless both-null row.
        //
        // `organization_id` is nullable to mirror `user_roles.organization_id`. A
        // global SuperAdmin grant has no organization, and those are the changes
        // most worth recording.
        //
        // Deliberately no foreign keys. An audit row must outlive the rows it
        // describes: CASCADE would let deleting a user erase the evidence they were
        // once an admin, and SET NULL would erase the actor, the most forensically
        // valuable field. `password_reset_attempts` omits them for the same reason.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS refactor_platform.user_role_changes (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    actor_user_id   UUID,
                    target_user_id  UUID NOT NULL,
                    organization_id UUID,
                    previous_role   refactor_platform.role,
                    new_role        refactor_platform.role,
                    changed_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    CONSTRAINT user_role_changes_not_both_null
                        CHECK (previous_role IS NOT NULL OR new_role IS NOT NULL)
                )",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE refactor_platform.user_role_changes OWNER TO refactor")
            .await?;

        // History of one member within one organization.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_user_role_changes_target_org_time \
                 ON refactor_platform.user_role_changes \
                 (target_user_id, organization_id, changed_at DESC)",
            )
            .await?;

        // Everything one admin did, across organizations.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_user_role_changes_actor_time \
                 ON refactor_platform.user_role_changes (actor_user_id, changed_at DESC)",
            )
            .await?;

        // Every change within one organization. The target-leading index above
        // cannot serve this, and an org-wide history is the obvious admin view.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_user_role_changes_org_time \
                 ON refactor_platform.user_role_changes (organization_id, changed_at DESC)",
            )
            .await?;

        // Make append-only a privilege, not just a convention. Nothing rewrites
        // these rows and there is no retention sweep, so the owner needs none of
        // the three. TRUNCATE is included because it would empty the table in one
        // statement while leaving DELETE denied. The owner can grant itself back,
        // so this stops accident and application compromise, not a determined
        // operator with database access.
        manager
            .get_connection()
            .execute_unprepared(
                "REVOKE UPDATE, DELETE, TRUNCATE ON refactor_platform.user_role_changes \
                 FROM refactor",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drops only the table. `refactor_platform.role` predates this migration and
        // is still the column type for `user_roles.role`.
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.user_role_changes")
            .await?;
        Ok(())
    }
}
