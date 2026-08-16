//! Role grants and removals.
//!
//! Every mutation in this module writes its own `user_role_changes` audit row,
//! which is why each takes an [`Actor`]. Reads are unaffected. The one
//! other writer of `user_roles` is `seed_database`, which is dev-only.
//!
//! Call the mutations inside a transaction: the audit row is written alongside
//! the change, and only a shared transaction makes the pair atomic.

use super::actor::Actor;
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

/// Deletes every role a user holds and audits each one.
///
/// Returns what was destroyed, so callers can report it without a second read.
/// This is the only path that can erase a global SuperAdmin grant.
pub async fn delete_by_user_id(
    db: &impl ConnectionTrait,
    actor: Actor,
    user_id: Id,
) -> Result<Vec<Model>, Error> {
    // Read first: afterwards there is nothing left to describe.
    let destroyed = find_by_user_id(db, user_id).await?;

    Entity::delete_many()
        .filter(Condition::all().add(Column::UserId.eq(user_id)))
        .exec(db)
        .await?;

    for role in &destroyed {
        crate::user_role_change::record(
            db,
            Some(actor),
            user_id,
            role.organization_id,
            Some(role.role.clone()),
            None,
        )
        .await?;
    }

    Ok(destroyed)
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
    actor: Actor,
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
    let granted = ActiveModel {
        user_id: Set(user_id),
        organization_id: Set(Some(organization_id)),
        role: Set(role.clone()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(|err| map_duplicate_role(err, organization_id))?;

    crate::user_role_change::record(
        db,
        Some(actor),
        user_id,
        Some(organization_id),
        None,
        Some(role),
    )
    .await?;

    Ok(granted)
}

/// Every role a user holds, across all organizations and including a global
/// SuperAdmin grant.
pub async fn find_by_user_id(db: &impl ConnectionTrait, user_id: Id) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .all(db)
        .await?)
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

/// Removes a membership the caller has already read, and audits it.
///
/// Takes the row rather than its keys so the audited role comes from the record
/// being deleted, and so callers that already fetched it (every caller does, to
/// guard the removal) need not read it twice.
///
/// Deletes by primary key. Filtering on `organization_id` would render
/// `organization_id = NULL` for a global SuperAdmin grant, which matches nothing
/// in Postgres, and the audit row would then claim a removal that never happened.
/// The row is only audited if it was actually deleted.
pub async fn delete(
    db: &impl ConnectionTrait,
    actor: Actor,
    membership: &Model,
) -> Result<(), Error> {
    let deleted = Entity::delete_by_id(membership.id).exec(db).await?;

    if deleted.rows_affected == 0 {
        return Ok(());
    }

    crate::user_role_change::record(
        db,
        Some(actor),
        membership.user_id,
        membership.organization_id,
        Some(membership.role.clone()),
        None,
    )
    .await
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

/// Keeps only those users who currently hold a role in `organization_id`.
///
/// Input order is preserved, so callers relying on a coach-then-coachee ordering
/// keep it. A global SuperAdmin is always kept.
pub async fn retain_organization_members(
    db: &impl ConnectionTrait,
    user_ids: &[Id],
    organization_id: Id,
) -> Result<Vec<Id>, Error> {
    let members: Vec<Id> = Entity::find()
        .select_only()
        .column(Column::UserId)
        .filter(Column::UserId.is_in(user_ids.iter().copied()))
        .filter(
            Condition::any()
                .add(Column::OrganizationId.eq(organization_id))
                .add(
                    Condition::all()
                        .add(Column::Role.eq(Role::SuperAdmin))
                        .add(Column::OrganizationId.is_null()),
                ),
        )
        .into_tuple::<Id>()
        .all(db)
        .await?;

    Ok(user_ids
        .iter()
        .copied()
        .filter(|user_id| members.contains(user_id))
        .collect())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod test {
    use super::*;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase};

    #[tokio::test]
    async fn test_delete_by_user_id() -> Result<(), Error> {
        // No roles come back, so the delete runs and there is nothing to audit.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([Vec::<Model>::new()])
            .into_connection();

        let user_id = Id::new_v4();
        let _ = delete_by_user_id(&db, Actor::new(Id::new_v4()), user_id).await;

        let sql: Vec<String> = db
            .into_transaction_log()
            .iter()
            .flat_map(|transaction| {
                transaction
                    .statements()
                    .iter()
                    .map(|statement| statement.sql.clone())
            })
            .collect();

        assert!(sql.iter().any(|statement| statement
            == r#"DELETE FROM "refactor_platform"."user_roles" WHERE "user_roles"."user_id" = $1"#));
        assert!(
            !sql.iter()
                .any(|statement| statement.contains("user_role_changes")),
            "no roles were destroyed, so nothing should be audited: {sql:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_role_tests.rs"]
mod tests;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_role_membership_tests.rs"]
mod membership_tests;
