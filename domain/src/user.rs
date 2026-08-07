use std::collections::HashMap;

use chrono::Utc;
use entity_api::error::{EntityApiErrorKind, Error as EntityApiError};
use entity_api::{
    coaching_relationship, coaching_session, mutate, query,
    query::{IntoQueryFilterMap, QuerySort},
    user, user_role,
};

use crate::{
    coaching_relationships,
    error::Error,
    error::{DomainErrorKind, EntityErrorKind, InternalErrorKind},
    magic_link_token, magic_link_tokens, users, Id,
};
pub use entity_api::{
    user::{
        create, find_by_email, find_by_id, find_by_ids, find_by_organization, generate_hash,
        verify_password, AuthSession, Backend, Credentials, Role,
    },
    user_roles,
};
use log::*;
use sea_orm::IntoActiveModel;
use sea_orm::{DatabaseConnection, TransactionTrait, Value};

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

    let active_model = existing_user.into_active_model();
    Ok(mutate::update::<users::ActiveModel, users::Column>(db, active_model, update_map).await?)
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
        Some(Value::String(Some(Box::new(generate_hash(password))))),
    );

    let active_model = existing_user.into_active_model();
    Ok(mutate::update::<users::ActiveModel, users::Column>(db, active_model, params).await?)
}

/// Creates a user in an organization and, when `coach_id` is given, their
/// coaching relationship, in a single transaction.
///
/// The two operations are deliberately combined: callers send the invitation
/// email once this returns, so a failed coach assignment has to leave nothing
/// behind. A rolled-back row is recoverable, a sent email is not.
///
/// # Errors
///
/// Any entity-layer error from the user or relationship creation, with nothing
/// committed.
pub async fn create_in_organization(
    db: &DatabaseConnection,
    organization_id: Id,
    user_model: users::Model,
    coach_id: Option<Id>,
) -> Result<users::Model, Error> {
    let txn = db.begin().await.map_err(EntityApiError::from)?;

    let new_user = user::create_by_organization(&txn, organization_id, user_model).await?;

    if let Some(coach_id) = coach_id {
        coaching_relationship::create(
            &txn,
            organization_id,
            new_coaching_relationship(coach_id, new_user.id),
        )
        .await?;
    }

    txn.commit().await.map_err(EntityApiError::from)?;

    Ok(new_user)
}

/// A relationship model for insertion. `id`, `slug` and `organization_id` are
/// set by `coaching_relationship::create`.
pub(crate) fn new_coaching_relationship(
    coach_id: Id,
    coachee_id: Id,
) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        coach_id,
        coachee_id,
        organization_id: Default::default(),
        id: Default::default(),
        slug: String::new(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

pub async fn delete(db: &DatabaseConnection, user_id: Id) -> Result<(), Error> {
    // This delete is global, so refuse it while the account is still reachable from
    // another organization. Callers should remove the membership instead.
    let organization_count = crate::user_role::count_organizations(db, user_id).await?;
    if organization_count > 1 {
        return Err(EntityApiError {
            source: None,
            error_kind: EntityApiErrorKind::UserBelongsToMultipleOrganizations {
                organization_count,
            },
        }
        .into());
    }

    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    coaching_relationship::delete_by_user_id(&txn, user_id).await?;
    user_role::delete_by_user_id(&txn, user_id).await?;
    user::delete(&txn, user_id).await?;

    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_tests.rs"]
mod tests;
