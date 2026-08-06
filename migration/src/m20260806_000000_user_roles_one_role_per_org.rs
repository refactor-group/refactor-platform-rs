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

/// A `(user_id, organization_id)` pair that holds more than one role.
struct DuplicatePair {
    user_id: String,
    organization_id: String,
    role_count: i64,
}

/// Refuses the migration when a pair already holds several roles.
///
/// Deleting the extra rows here would silently change someone's effective
/// privileges, so the operator resolves each pair by hand instead.
fn duplicate_pairs_error(pairs: &[DuplicatePair]) -> Option<DbErr> {
    (!pairs.is_empty()).then(|| {
        let offenders = pairs
            .iter()
            .map(|pair| {
                format!(
                    "(user_id={}, organization_id={}, roles={})",
                    pair.user_id, pair.organization_id, pair.role_count
                )
            })
            .collect::<Vec<String>>()
            .join(", ");

        DbErr::Custom(format!(
            "Cannot create unique index user_roles_user_org_unique: {} user/organization pair(s) \
             hold more than one role: {offenders}. Reduce each pair to the single intended role \
             before running this migration.",
            pairs.len()
        ))
    })
}

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

        let duplicate_pairs = conn
            .query_all(Statement::from_string(backend, DUPLICATE_PAIRS_SQL))
            .await?
            .into_iter()
            .map(|row| {
                Ok(DuplicatePair {
                    user_id: row.try_get("", "user_id")?,
                    organization_id: row.try_get("", "organization_id")?,
                    role_count: row.try_get("", "role_count")?,
                })
            })
            .collect::<Result<Vec<DuplicatePair>, DbErr>>()?;

        if let Some(err) = duplicate_pairs_error(&duplicate_pairs) {
            return Err(err);
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

#[cfg(test)]
#[path = "m20260806_000000_user_roles_one_role_per_org_tests.rs"]
mod tests;
