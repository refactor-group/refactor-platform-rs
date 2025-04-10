use crate::{
    error::Error,
    error::{DomainErrorKind, EntityErrorKind, InternalErrorKind},
    users, Id,
};
use chrono::Utc;
pub use entity_api::user::{
    create, create_by_organization, find_by_email, find_by_id, find_by_organization,
    verify_password, AuthSession, Backend, Credentials,
};
use entity_api::{
    coaching_relationship, mutate, organizations_user, query, query::IntoQueryFilterMap, user,
};
use sea_orm::IntoActiveModel;
use sea_orm::{DatabaseConnection, TransactionTrait, Value};

pub async fn find_by(
    db: &DatabaseConnection,
    params: impl IntoQueryFilterMap,
) -> Result<Vec<users::Model>, Error> {
    let users =
        query::find_by::<users::Entity, users::Column>(db, params.into_query_filter_map()).await?;

    Ok(users)
}

pub async fn update(
    db: &DatabaseConnection,
    user_id: Id,
    params: impl mutate::IntoUpdateMap,
) -> Result<users::Model, Error> {
    let existing_user = find_by_id(db, user_id).await?;

    let mut params = params.into_update_map();

    // Extract and verify the user's password as a security check before allowing any updates
    let password_to_verify = remove_from_params(&mut params, "password").await?;
    verify_password(&password_to_verify, &existing_user.password).await?;

    // After verification passes, proceed with the update
    let active_model = existing_user.into_active_model();
    Ok(mutate::update::<users::ActiveModel, users::Column>(db, active_model, params).await?)
}

pub async fn update_password(
    db: &DatabaseConnection,
    user_id: Id,
    params: impl mutate::IntoUpdateMap,
) -> Result<users::Model, Error> {
    let existing_user = find_by_id(db, user_id).await?;
    let mut params = params.into_update_map();
    // Extract and verify the user's password as a security check before allowing any updates
    let password_to_verify = remove_from_params(&mut params, "current_password").await?;
    // Also check that the confirm password matches
    let confirm_password = remove_from_params(&mut params, "confirm_password").await?;

    if confirm_password != password_to_verify {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
        });
    }
    verify_password(&password_to_verify, &existing_user.password).await?;
    // After verification passes, proceed with the update
    let active_model = existing_user.into_active_model();
    Ok(mutate::update::<users::ActiveModel, users::Column>(db, active_model, params).await?)
}

// This function is intended to be a temporary solution until we finalize our user experience strategy for assigning a new user
// to a coach or designating them as a coach. In the future, the API will require the frontend to make separate requests:
// one request to create a new user within the scope of an organization, and a subsequent request to assign that user to a
// coaching relationship. This separation is necessary because a user can be created and then assigned to a coaching relationship at
// a later time. Currently, we are combining these two operations to leverage the backend database transaction, which helps
// prevent inconsistencies or errors that might arise from network issues or other problems, ensuring a consistent state
// between new users and their coaching relationships.
pub async fn create_user_and_coaching_relationship(
    db: &DatabaseConnection,
    organization_id: Id,
    coach_id: Id,
    user_model: users::Model,
) -> Result<users::Model, Error> {
    // This is not probably the type of error we'll ultimately be exposing. Again just temporary (hopfully)
    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Other)),
    })?;

    // Create the user within the organization
    let new_user = create_by_organization(&txn, organization_id, user_model).await?;
    // Create the coaching relationship using the new user's ID as the coachee_id
    let new_coaching_relationship_model = entity_api::coaching_relationships::Model {
        coachee_id: new_user.id,
        coach_id,
        organization_id,
        // These will be overridden
        id: Default::default(),
        slug: "".to_string(),
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    };
    entity_api::coaching_relationship::create(&txn, new_coaching_relationship_model).await?;
    // This is not probably the type of error we'll ultimately be exposing. Again just temporary (hopfully
    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Other)),
    })?;
    Ok(new_user)
}

pub async fn delete(db: &DatabaseConnection, user_id: Id) -> Result<(), Error> {
    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Other)),
    })?;

    coaching_relationship::delete_by_user_id(&txn, user_id).await?;
    organizations_user::delete_by_user_id(&txn, user_id).await?;
    user::delete(&txn, user_id).await?;

    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Other)),
    })?;

    Ok(())
}

/// Extracts the password from the update parameters.
/// First removes the "password" field from the params map, then ensures it's a valid string value.
/// Returns the password as a String if found and valid, otherwise returns an Internal Error.
async fn remove_from_params(params: &mut mutate::UpdateMap, key: &str) -> Result<String, Error> {
    params
        .remove(key)
        .ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
        })
        .and_then(|v| match v {
            Value::String(Some(boxed_str)) => Ok((*boxed_str).clone()),
            _ => Err(Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
            }),
        })
}
