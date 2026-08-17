//! Organization membership: attaching, removing, and scoped lookup of users.
//!
//! A membership is a `user_roles` row. A user may hold rows in several
//! organizations, so these operations are always scoped to one organization and
//! never touch the `users` row itself.

use entity_api::error::{EntityApiErrorKind, Error as EntityApiError};
use entity_api::{coaching_relationship, organization, user, user_role};
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::Error;
use crate::user::{new_coaching_relationship, Role};
use crate::{user_roles, users, Actor, Id};

/// Minimal projection of a user returned by an email lookup.
///
/// Deliberately narrow: the requester may know nothing about this user beyond
/// their email, so roles, invite status, timezone and profile links stay out.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct UserLookupResult {
    pub id: Id,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
}

/// Grants an existing user a role in an organization, optionally pre-assigning
/// them a coach in the same transaction.
///
/// The coaching relationship shares the membership's transaction so that callers
/// can notify the user only once both have committed.
///
/// # Errors
///
/// `NotFound` when the organization or user does not exist, `OrganizationArchived`
/// when the organization is archived, `UserAlreadyInOrganization` when the user
/// already holds a role there, plus any error from the coach assignment.
pub async fn attach_to_organization(
    db: &DatabaseConnection,
    actor: Actor,
    organization_id: Id,
    user_id: Id,
    role: Role,
    coach_id: Option<Id>,
) -> Result<users::Model, Error> {
    let txn = db.begin().await.map_err(EntityApiError::from)?;

    let organization = organization::find_by_id(&txn, organization_id).await?;
    if organization.archived_at.is_some() {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationArchived,
        }
        .into());
    }

    // Held for the rest of the transaction so a concurrent account deletion
    // cannot read this user's memberships before the new row lands and then
    // delete it along with the rest.
    user::find_by_id_for_update(&txn, user_id).await?;

    if user_role::find_by_user_and_organization(&txn, user_id, organization_id)
        .await?
        .is_some()
    {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::UserAlreadyInOrganization { organization_id },
        }
        .into());
    }

    user_role::create(&txn, actor, user_id, organization_id, role).await?;

    if let Some(coach_id) = coach_id {
        coaching_relationship::create(
            &txn,
            organization_id,
            new_coaching_relationship(coach_id, user_id),
        )
        .await?;
    }

    let mut attached_user = user::find_by_id(&txn, user_id).await?;
    user::scope_roles_to_organization(&mut attached_user, organization_id);

    txn.commit().await.map_err(EntityApiError::from)?;

    Ok(attached_user)
}

/// Removes a user from one organization, leaving their account and other
/// memberships intact.
///
/// Only the membership row is deleted. The user's coaching relationships and
/// sessions in the organization are deliberately preserved; authorization
/// denies the removed user access to them.
///
/// Returns the role that was removed, so callers can log the transition.
///
/// # Errors
///
/// `NotFound` when the user holds no role in the organization,
/// `LastOrganizationAdmin` when they are its only admin.
pub async fn remove_from_organization(
    db: &DatabaseConnection,
    actor: Actor,
    organization_id: Id,
    user_id: Id,
) -> Result<Role, Error> {
    let txn = db.begin().await.map_err(EntityApiError::from)?;

    let Some(membership) =
        user_role::find_by_user_and_organization(&txn, user_id, organization_id).await?
    else {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        }
        .into());
    };

    if membership.role == Role::Admin
        && user_role::count_admins_in_organization(&txn, organization_id).await? <= 1
    {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::LastOrganizationAdmin { organization_id },
        }
        .into());
    }

    user_role::delete(&txn, actor, &membership).await?;

    txn.commit().await.map_err(EntityApiError::from)?;

    Ok(membership.role)
}

/// Reads the role a user holds in one organization.
///
/// A single read, so it takes any connection rather than opening a transaction.
///
/// # Errors
///
/// `NotFound` when the user holds no role in the organization.
pub async fn find_role_in_organization(
    db: &impl ConnectionTrait,
    organization_id: Id,
    user_id: Id,
) -> Result<user_roles::Model, Error> {
    let Some(membership) =
        user_role::find_by_user_and_organization(db, user_id, organization_id).await?
    else {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        }
        .into());
    };

    Ok(membership)
}

/// Changes the role a user holds in one organization, in place.
///
/// Atomic where a remove-then-add pair is not, and it leaves the user's coaching
/// relationships and sessions untouched.
///
/// # Errors
///
/// `NotFound` when the organization or the user does not exist, or the user holds
/// no role in the organization; `OrganizationArchived` when the organization is
/// archived; `LastOrganizationAdmin` when demoting the organization's only admin;
/// `ValidationError` for `Role::SuperAdmin`.
pub async fn update_role_in_organization(
    db: &DatabaseConnection,
    actor: Actor,
    organization_id: Id,
    user_id: Id,
    role: Role,
) -> Result<users::Model, Error> {
    let txn = db.begin().await.map_err(EntityApiError::from)?;

    let organization = organization::find_by_id(&txn, organization_id).await?;
    if organization.archived_at.is_some() {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationArchived,
        }
        .into());
    }

    // Taken before the membership read and before the admin count, so this path
    // locks users then user_roles like account deletion does and cannot deadlock.
    user::find_by_id_for_update(&txn, user_id).await?;

    let Some(membership) =
        user_role::find_by_user_and_organization(&txn, user_id, organization_id).await?
    else {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        }
        .into());
    };

    // Only a demotion can leave the organization unadministrable, and the count
    // locks every admin row, so a promotion must not pay for it.
    if membership.role == Role::Admin
        && role != Role::Admin
        && user_role::count_admins_in_organization(&txn, organization_id).await? <= 1
    {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::LastOrganizationAdmin { organization_id },
        }
        .into());
    }

    // Setting the role a member already holds is an idempotent no-op: no update,
    // no audit row, and the same response as a real change.
    if membership.role != role {
        user_role::update_role(&txn, actor, &membership, role).await?;
    }

    let mut updated_user = user::find_by_id(&txn, user_id).await?;
    user::scope_roles_to_organization(&mut updated_user, organization_id);

    txn.commit().await.map_err(EntityApiError::from)?;

    Ok(updated_user)
}

/// Looks up a user by email, limited to what the requester is allowed to see.
///
/// Returns 0 or 1 results; an empty vector is the not-found signal, so a caller
/// cannot tell "no such email" from "a user you may not see".
pub async fn lookup_by_email_scoped(
    db: &DatabaseConnection,
    requester: &users::Model,
    email: &str,
) -> Result<Vec<UserLookupResult>, Error> {
    let found = user::find_by_email_ci(db, email.trim()).await?;

    let requester_is_super_admin = requester
        .roles
        .iter()
        .any(|role| role.role == Role::SuperAdmin && role.organization_id.is_none());

    // Run the scope check on every path, including unknown emails and super admin
    // requesters, so response timing cannot be used to enumerate accounts.
    let shares_organization = user_role::shares_administered_organization(
        db,
        requester.id,
        found.as_ref().map_or_else(Id::nil, |user| user.id),
    )
    .await?;

    Ok(found
        .filter(|_| requester_is_super_admin || shares_organization)
        .map(|user| UserLookupResult {
            id: user.id,
            first_name: user.first_name,
            last_name: user.last_name,
            email: user.email,
        })
        .into_iter()
        .collect())
}

/// Whether `requester` may act on `target_user_id`.
///
/// True for a global SuperAdmin, or when the requester administers an
/// organization the target belongs to. Unlike `lookup_by_email_scoped` this may
/// short-circuit: the target id is already known to the caller, so query count
/// reveals nothing.
pub async fn can_administer_user(
    db: &impl ConnectionTrait,
    requester: &users::Model,
    target_user_id: Id,
) -> Result<bool, Error> {
    let requester_is_super_admin = requester
        .roles
        .iter()
        .any(|role| role.role == Role::SuperAdmin && role.organization_id.is_none());

    Ok(requester_is_super_admin
        || user_role::shares_administered_organization(db, requester.id, target_user_id).await?)
}

/// Counts the distinct organizations a user belongs to.
pub async fn count_organizations(db: &impl ConnectionTrait, user_id: Id) -> Result<u64, Error> {
    Ok(user_role::count_organizations_for_user(db, user_id).await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_role_tests.rs"]
mod tests;
