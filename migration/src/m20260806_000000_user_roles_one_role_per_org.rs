use sea_orm::Statement;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const DUPLICATE_PAIRS_SQL: &str = r#"
    SELECT user_id::text AS user_id,
           organization_id::text AS organization_id,
           count(*) AS role_count
    FROM refactor_platform.user_roles
    WHERE organization_id IS NOT NULL
    GROUP BY user_id, organization_id
    HAVING count(*) > 1
    ORDER BY user_id, organization_id
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A user is meant to hold exactly one role per organization, but the old
        // index keyed on `role` too, so `(user, org, User)` and `(user, org, Admin)`
        // could coexist. Role resolution then depends on row order, and one of the
        // answers is Admin. The database now rejects the second grant, closing the
        // race between two concurrent attach requests carrying different roles.
        let conn = manager.get_connection();
        let backend = conn.get_database_backend();

        // Deleting the extra rows here would silently change someone's effective
        // privileges, so name them and let the operator resolve each by hand.
        let duplicates = conn
            .query_all(Statement::from_string(backend, DUPLICATE_PAIRS_SQL))
            .await?
            .into_iter()
            .map(|row| {
                Ok(format!(
                    "(user_id={}, organization_id={}, roles={})",
                    row.try_get::<String>("", "user_id")?,
                    row.try_get::<String>("", "organization_id")?,
                    row.try_get::<i64>("", "role_count")?
                ))
            })
            .collect::<Result<Vec<String>, DbErr>>()?;

        if !duplicates.is_empty() {
            return Err(DbErr::Custom(format!(
                "Cannot create unique index user_roles_user_org_unique: {} user/organization \
                 pair(s) hold more than one role: {}. Reduce each pair to the single intended \
                 role before running this migration.",
                duplicates.len(),
                duplicates.join(", ")
            )));
        }

        conn.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS user_roles_user_org_unique \
             ON refactor_platform.user_roles(user_id, organization_id) \
             WHERE organization_id IS NOT NULL",
        )
        .await?;

        // The new index strictly subsumes the three-column one, and its
        // (user_id, organization_id) prefix serves the same lookups.
        conn.execute_unprepared(
            "DROP INDEX IF EXISTS refactor_platform.user_roles_user_org_role_unique",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS user_roles_user_org_role_unique \
             ON refactor_platform.user_roles(user_id, organization_id, role) \
             WHERE organization_id IS NOT NULL",
        )
        .await?;

        conn.execute_unprepared(
            "DROP INDEX IF EXISTS refactor_platform.user_roles_user_org_unique",
        )
        .await?;

        Ok(())
    }
}
