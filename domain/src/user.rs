use crate::{error::Error, users, Id};
use entity_api::mutate;
use sea_orm::DatabaseConnection;
use sea_orm::IntoActiveModel;

pub use entity_api::user::{create, find_by_email, find_by_id};

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
