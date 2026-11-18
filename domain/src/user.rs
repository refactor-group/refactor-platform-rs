use std::collections::HashMap;

use chrono::Weekday;
use entity_api::error::{EntityApiErrorKind, Error as EntityApiError};
use entity_api::{
    coaching_relationship, coaching_session, mutate, query,
    query::{IntoQueryFilterMap, QuerySort},
    user, user_role,
};

use crate::{
    audit_event::{self, Recorder},
    coaching_relationship::{create_for_invitee, InviteeRelationship},
    error::Error,
    error::{reason, DomainErrorKind, EntityErrorKind, InternalErrorKind},
    magic_link_token, magic_link_tokens, users, Actor, Id,
};
pub use entity_api::{
    user::{
        create, find_by_email, find_by_id, find_by_id_without_roles, find_by_ids,
        find_by_organization, generate_hash, parse_working_hours, verify_password, AuthSession,
        Backend, Credentials, Role, WorkingHoursPatch,
    },
    user_roles,
};
use log::*;
use sea_orm::IntoActiveModel;
use sea_orm::{DatabaseConnection, TransactionTrait, Value};

/// The last day of the week for anyone who has not chosen their own.
pub const DEFAULT_WEEK_END: Weekday = Weekday::Fri;

/// The last day of `user`'s week; the default until users can choose their own.
pub fn week_end(_user: &users::Model) -> Weekday {
    DEFAULT_WEEK_END
}

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<users::Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<users::Column>,
{
    let users = query::find_by::<users::Entity, users::Column, P>(db, params).await?;
    Ok(users)
}

pub async fn find_by_organization_with_invite_status(
    db: &DatabaseConnection,
    organization_id: Id,
) -> Result<Vec<users::Model>, Error> {
    let mut users = user::find_by_organization(db, organization_id).await?;

    if users.is_empty() {
        return Ok(users);
    }

    let user_ids: Vec<Id> = users.iter().map(|u| u.id).collect();
    let tokens = entity_api::magic_link_token::find_by_user_ids(
        db,
        &user_ids,
        crate::token_purpose::TokenPurpose::Setup,
    )
    .await?;

    let token_map: HashMap<Id, &magic_link_tokens::Model> =
        tokens.iter().map(|t| (t.user_id, t)).collect();

    for user in &mut users {
        user.invite_status = Some(magic_link_token::compute_invite_status(
            &user.password,
            token_map.get(&user.id).copied(),
        ));
    }

    Ok(users)
}

pub async fn update(
    db: &DatabaseConnection,
    user_id: Id,
    params: impl mutate::IntoUpdateMap,
) -> Result<users::Model, Error> {
    let update_map = params.into_update_map();
    // Validate `default_coaching_session_duration_minutes` if present.
    // The IntoUpdateMap pattern erases types, so we re-check the range
    // (1..=480) at the entity_api boundary.
    coaching_session::validate_duration_in_update_map(
        &update_map,
        "default_coaching_session_duration_minutes",
    )?;

    let existing_user = find_by_id(db, user_id).await?;
    user::validate_working_hours_in_update_map(&update_map, &existing_user)?;

    Ok(user::update_fields(db, existing_user, update_map).await?)
}

pub async fn update_password(
    db: &DatabaseConnection,
    user_id: Id,
    params: impl mutate::IntoUpdateMap,
) -> Result<users::Model, Error> {
    let existing_user = find_by_id(db, user_id).await?;
    let mut params = params.into_update_map();

    // Remove and verify the user's current password as a security check before allowing any updates
    let password_to_verify = params.remove("current_password")?;
    verify_password(&password_to_verify, existing_user.password.as_deref()).await?;

    // remove confirm_password
    let confirm_password = params.remove("confirm_password")?;

    // remove password
    let password = params.remove("password")?;
    // check password confirmation
    if confirm_password != password {
        warn!("Password confirmation does not match");
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation(
                "Password confirmation does not match".to_string(),
            ),
        });
    }

    // generate new password hash and insert it back into params overwriting the raw password
    params.insert(
        "password".to_string(),
        Some(Value::String(Some(generate_hash(password)))),
    );

    let active_model = existing_user.into_active_model();
    Ok(mutate::update::<users::ActiveModel, users::Column>(db, active_model, params).await?)
}

/// Creates a user in an organization and, when `relationship` is given, their
/// relationship with that person, in a single transaction.
///
/// The two operations are deliberately combined: callers send the invitation
/// email once this returns, so a refused relationship has to leave nothing
/// behind. A rolled-back row is recoverable, a sent email is not.
///
/// # Errors
///
/// A 422 (`role_not_in_kind`) before anything is written when `relationship.role` is
/// no role of its kind; otherwise any entity-layer error from the user or relationship
/// creation, with nothing committed.
pub async fn create_in_organization(
    db: &DatabaseConnection,
    recorder: &dyn Recorder,
    actor: Actor,
    organization_id: Id,
    user_model: users::Model,
    relationship: Option<InviteeRelationship>,
) -> Result<users::Model, Error> {
    let relationship = relationship.map(InviteeRelationship::seated).transpose()?;

    let txn = db.begin().await.map_err(EntityApiError::from)?;

    let new_user = user::create_by_organization(&txn, actor, organization_id, user_model).await?;
    let relationship_event =
        create_for_invitee(&txn, actor, organization_id, new_user.id, relationship).await?;

    let events = new_user
        .roles
        .iter()
        .map(|granted| audit_event::member_added(actor, organization_id, new_user.id, granted.role))
        .chain(relationship_event)
        .collect();
    recorder.record_all(&txn, events).await?;

    txn.commit().await.map_err(EntityApiError::from)?;

    Ok(new_user)
}

/// Deletes an account and every role it holds.
///
/// Refused with a 409 while any of the account's relationships has a coaching
/// session; relationships without one are deleted with it.
///
/// Returns the roles that were destroyed, so the caller can log them. Each is
/// also written to the audit trail inside the transaction. A global SuperAdmin
/// grant is among them, and that is the change most worth recording. After the
/// deletes, the audit log records them organization by organization: in each, every
/// deleted relationship, then the user's removal from it.
pub async fn delete(
    db: &DatabaseConnection,
    recorder: &dyn Recorder,
    actor: Actor,
    user_id: Id,
) -> Result<Vec<user_roles::Model>, Error> {
    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    // Both guards below read this user's memberships and then delete every one of
    // them, so a membership committed in between would be destroyed unchecked.
    // Attaching a membership takes the same lock.
    user::find_by_id_for_update(&txn, user_id).await?;

    // This delete is global, so refuse it while the account is still reachable from
    // another organization. Callers should remove the membership instead.
    let organization_count = crate::user_role::count_organizations(&txn, user_id).await?;
    if organization_count > 1 {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::UserBelongsToMultipleOrganizations {
                organization_count,
            },
        }
        .into());
    }

    // Deleting the account drops its role rows, so it owes the organization the
    // same last-admin invariant the remove-from-organization path enforces.
    // Reachable by a super admin, who passes the org gate without being an admin
    // of the organization themselves.
    for organization_id in user_role::find_administered_organizations(&txn, user_id).await? {
        if user_role::count_admins_in_organization(&txn, organization_id).await? <= 1 {
            return Err(EntityApiError {
                source: None,
                error_kind: EntityApiErrorKind::LastOrganizationAdmin { organization_id },
            }
            .into());
        }
    }

    // Sessions block the relationship delete, and the shared history must survive.
    let relationship_count =
        coaching_relationship::count_with_sessions_by_user_id(&txn, user_id).await?;
    if relationship_count > 0 {
        return Err(coaching_history(user_id, Some(relationship_count)));
    }

    // A session created since the count still trips the foreign key. Postgres has aborted
    // the transaction by then, so the count cannot be redone.
    let deleted_relationships = coaching_relationship::delete_by_user_id(&txn, user_id)
        .await
        .map_err(|err| {
            if err.is_foreign_key_violation() {
                coaching_history(user_id, None)
            } else {
                err.into()
            }
        })?;
    let destroyed_roles = user_role::delete_by_user_id(&txn, actor, user_id).await?;
    user::delete(&txn, user_id).await?;

    let events = deleted_relationships
        .iter()
        .map(|relationship| audit_event::relationship_deleted(actor, relationship))
        .chain(
            destroyed_roles
                .iter()
                .filter_map(|role| role.organization_id)
                .map(|organization_id| {
                    audit_event::member_removed(actor, organization_id, user_id)
                }),
        )
        .collect();
    recorder.record_all(&txn, events).await?;

    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    Ok(destroyed_roles)
}

/// A 409 refusing to delete an account with coaching history; `relationship_count` is
/// how many of its relationships have sessions, when known.
fn coaching_history(user_id: Id, relationship_count: Option<u64>) -> Error {
    let mut details = serde_json::json!({ "user_id": user_id });
    if let Some(count) = relationship_count {
        details["relationship_count"] = count.into();
    }
    Error::conflict_because(
        reason::USER_HAS_COACHING_HISTORY,
        "This person has coaching history, so remove them from the organization instead of deleting their account.",
        details,
    )
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "user_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(test)]
#[path = "user_delete_sqlite_tests.rs"]
mod delete_sqlite_tests;
