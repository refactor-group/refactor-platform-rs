use crate::{error::Error, users, Id};
use entity_api::{mutate, query, query::IntoQueryFilterMap};
use sea_orm::DatabaseConnection;
use sea_orm::IntoActiveModel;

pub use entity_api::user::{
    create, create_by_organization, find_by_email, find_by_id, find_by_organization, AuthSession,
    Backend, Credentials,
};

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
    let active_model = existing_user.into_active_model();
    Ok(mutate::update::<users::ActiveModel, users::Column>(
        db,
        active_model,
        params.into_update_map(),
    )
    .await?)
}
