use super::error::{EntityApiErrorKind, Error};
use chrono::Utc;
use entity::roles::Role;
use entity::user_roles::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QuerySelect, Set, SqlErr,
};

/// Partial unique index holding a user to one role per organization.
const ONE_ROLE_PER_ORGANIZATION_INDEX: &str = "user_roles_user_org_unique";

pub async fn delete_by_user_id(db: &impl ConnectionTrait, user_id: Id) -> Result<(), Error> {
    Entity::delete_many()
        .filter(Condition::all().add(Column::UserId.eq(user_id)))
        .exec(db)
        .await?;
    Ok(())
}

/// Translates a violation of the one-role-per-organization index into a 409.
///
/// Only that index is recognised, so an unrelated unique violation keeps its
/// `SystemError` mapping instead of being reported as a membership conflict.
fn map_duplicate_role(err: DbErr, organization_id: Id) -> Error {
    if is_one_role_per_organization_violation(err.sql_err()) {
        return Error {
            source: Some(err),
            error_kind: EntityApiErrorKind::UserAlreadyInOrganization { organization_id },
        };
    }
    Error::from(err)
}

/// Whether a database error is a violation of the one-role-per-organization index.
///
/// Split out from [`map_duplicate_role`] so the constraint-name policy is
/// testable: a `DbErr` carrying a real `SqlErr` cannot be constructed by hand.
fn is_one_role_per_organization_violation(sql_err: Option<SqlErr>) -> bool {
    matches!(
        sql_err,
        Some(SqlErr::UniqueConstraintViolation(constraint))
            if constraint.contains(ONE_ROLE_PER_ORGANIZATION_INDEX)
    )
}

/// Grants `role` to `user_id` within `organization_id`.
///
/// # Errors
///
/// Returns `ValidationError` for `Role::SuperAdmin`, which is a global role and
/// cannot be scoped to an organization. Rejected before any query runs so the
/// caller gets a 422 rather than the 500 the entity-level `before_save` guard
/// would produce.
///
/// Returns `UserAlreadyInOrganization` when the user already holds a role there,
/// including when two concurrent grants race past the application-level check.
pub async fn create(
    db: &impl ConnectionTrait,
    user_id: Id,
    organization_id: Id,
    role: Role,
) -> Result<Model, Error> {
    if role == Role::SuperAdmin {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::ValidationError {
                message: "SuperAdmin cannot be granted within an organization.".into(),
                details: None,
            },
        });
    }

    let now = Utc::now();
    ActiveModel {
        user_id: Set(user_id),
        organization_id: Set(Some(organization_id)),
        role: Set(role),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(|err| map_duplicate_role(err, organization_id))
}

/// Finds the role a user holds in a specific organization, if any.
pub async fn find_by_user_and_organization(
    db: &impl ConnectionTrait,
    user_id: Id,
    organization_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::OrganizationId.eq(organization_id))
        .one(db)
        .await?)
}

/// Removes a user's role in one organization, returning the number of rows deleted.
pub async fn delete_by_user_and_organization(
    db: &impl ConnectionTrait,
    user_id: Id,
    organization_id: Id,
) -> Result<u64, Error> {
    Ok(Entity::delete_many()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::OrganizationId.eq(organization_id))
        .exec(db)
        .await?
        .rows_affected)
}

/// Counts the distinct organizations a user belongs to, ignoring global roles.
pub async fn count_organizations_for_user(
    db: &impl ConnectionTrait,
    user_id: Id,
) -> Result<u64, Error> {
    Ok(Entity::find()
        .select_only()
        .column(Column::OrganizationId)
        .distinct()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::OrganizationId.is_not_null())
        .into_tuple::<Option<Id>>()
        .count(db)
        .await?)
}

/// Organizations in which the user holds the Admin role.
pub async fn find_administered_organizations(
    db: &impl ConnectionTrait,
    user_id: Id,
) -> Result<Vec<Id>, Error> {
    Ok(Entity::find()
        .select_only()
        .column(Column::OrganizationId)
        .filter(Column::UserId.eq(user_id))
        .filter(Column::Role.eq(Role::Admin))
        .filter(Column::OrganizationId.is_not_null())
        .into_tuple::<Option<Id>>()
        .all(db)
        .await?
        .into_iter()
        .flatten()
        .collect())
}

/// Counts the admins of an organization, locking those rows for the caller's
/// transaction.
///
/// The lock is what makes the last-admin guard safe. Read committed lets two
/// concurrent removals both observe two admins and both commit, leaving the
/// organization with none; taking `FOR UPDATE` here makes the second removal
/// wait and then re-read a count of one. Postgres rejects `FOR UPDATE`
/// alongside an aggregate, so the rows are counted in Rust rather than by
/// `COUNT(*)`.
///
/// Callers must run inside a transaction for the lock to outlive this call.
pub async fn count_admins_in_organization(
    db: &impl ConnectionTrait,
    organization_id: Id,
) -> Result<u64, Error> {
    Ok(Entity::find()
        .filter(Column::OrganizationId.eq(organization_id))
        .filter(Column::Role.eq(Role::Admin))
        .lock_exclusive()
        .all(db)
        .await?
        .len() as u64)
}

/// Whether `requester_id` administers an organization that `target_user_id` belongs to.
///
/// This is the visibility predicate for cross-organization user lookups: being a
/// plain member of a shared organization is not enough, the requester must hold
/// `Role::Admin` there. Global SuperAdmin rows (`organization_id IS NULL`) are
/// ignored; that bypass belongs one layer up.
///
/// Always issues exactly two queries so callers can rely on a constant query
/// count regardless of the answer.
pub async fn shares_administered_organization(
    db: &impl ConnectionTrait,
    requester_id: Id,
    target_user_id: Id,
) -> Result<bool, Error> {
    let administered_organization_ids = Entity::find()
        .select_only()
        .column(Column::OrganizationId)
        .filter(Column::UserId.eq(requester_id))
        .filter(Column::Role.eq(Role::Admin))
        .filter(Column::OrganizationId.is_not_null())
        .into_tuple::<Option<Id>>()
        .all(db)
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<Id>>();

    Ok(Entity::find()
        .select_only()
        .column(Column::Id)
        .filter(Column::UserId.eq(target_user_id))
        .filter(Column::OrganizationId.is_in(administered_organization_ids))
        .into_tuple::<Id>()
        .one(db)
        .await?
        .is_some())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod test {
    use super::*;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn test_delete_by_user_id() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = delete_by_user_id(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"DELETE FROM "refactor_platform"."user_roles" WHERE "user_roles"."user_id" = $1"#,
                [user_id.into()]
            )]
        );

        Ok(())
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_role_tests.rs"]
mod tests;
