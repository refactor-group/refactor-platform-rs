use super::error::Error;

use chrono::Utc;
use entity::magic_link_tokens::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::{entity::prelude::*, ConnectionTrait, Set};

/// Insert a new magic link token row.
pub async fn create(
    db: &impl ConnectionTrait,
    user_id: Id,
    token_hash: String,
    expires_at: DateTimeWithTimeZone,
) -> Result<Model, Error> {
    let now = Utc::now();

    let active_model = ActiveModel {
        user_id: Set(user_id),
        token_hash: Set(token_hash),
        expires_at: Set(expires_at),
        created_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.insert(db).await?)
}

/// Look up a magic link token by its SHA-256 hash.
pub async fn find_by_token_hash(
    db: &impl ConnectionTrait,
    token_hash: &str,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::TokenHash.eq(token_hash))
        .one(db)
        .await?)
}

/// Fetch the magic link token for each of the given user IDs (at most one per user).
pub async fn find_by_user_ids(
    db: &impl ConnectionTrait,
    user_ids: &[Id],
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.is_in(user_ids.to_vec()))
        .all(db)
        .await?)
}

/// Delete all magic link tokens for a given user.
pub async fn delete_all_for_user(db: &impl ConnectionTrait, user_id: Id) -> Result<(), Error> {
    Entity::delete_many()
        .filter(Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    Ok(())
}
