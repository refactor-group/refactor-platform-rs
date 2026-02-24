use crate::{
    error::Error,
    error::{DomainErrorKind, EntityErrorKind, InternalErrorKind},
    users, Id,
};
use chrono::Utc;
use entity_api::{
    coaching_relationship, mutate, query,
    query::{IntoQueryFilterMap, QuerySort},
    user, user_role,
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

pub async fn update(
    db: &DatabaseConnection,
    user_id: Id,
    params: impl mutate::IntoUpdateMap,
) -> Result<users::Model, Error> {
    let existing_user = find_by_id(db, user_id).await?;

    let active_model = existing_user.into_active_model();
    Ok(mutate::update::<users::ActiveModel, users::Column>(
        db,
        active_model,
        params.into_update_map(),
    )
    .await?)
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
    verify_password(&password_to_verify, &existing_user.password).await?;

    // remove confirm_password
    let confirm_password = params.remove("confirm_password")?;

    // remove password
    let password = params.remove("password")?;
    // check password confirmation
    if confirm_password != password {
        warn!("Password confirmation does not match");
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Password confirmation does not match".to_string(),
            )),
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
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    // Create the user within the organization
    let new_user =
        entity_api::user::create_by_organization(&txn, organization_id, user_model).await?;
    // Create the coaching relationship using the new user's ID as the coachee_id
    let new_coaching_relationship_model = entity_api::coaching_relationships::Model {
        coachee_id: new_user.id,
        coach_id,
        // These will be overridden
        organization_id: Default::default(),
        id: Default::default(),
        slug: "".to_string(),
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    };
    entity_api::coaching_relationship::create(
        &txn,
        organization_id,
        new_coaching_relationship_model,
    )
    .await?;
    // This is not probably the type of error we'll ultimately be exposing. Again just temporary (hopfully
    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;
    Ok(new_user)
}

pub async fn delete(db: &DatabaseConnection, user_id: Id) -> Result<(), Error> {
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

pub async fn create_by_organization(
    db: &DatabaseConnection,
    organization_id: Id,
    user_model: users::Model,
) -> Result<users::Model, Error> {
    // Create the user first using the entity_api function
    let new_user =
        entity_api::user::create_by_organization(db, organization_id, user_model).await?;

    Ok(new_user)
}
