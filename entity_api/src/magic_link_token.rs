use super::error::Error;

use chrono::Utc;
use entity::magic_link_tokens::{ActiveModel, Column, Entity, Model};
use entity::token_purpose::TokenPurpose;
use entity::Id;
use sea_orm::{entity::prelude::*, ConnectionTrait, QueryOrder, Set};

/// Insert a new magic link token row.
pub async fn create(
    db: &impl ConnectionTrait,
    user_id: Id,
    token_hash: String,
    expires_at: DateTimeWithTimeZone,
    purpose: TokenPurpose,
) -> Result<Model, Error> {
    let now = Utc::now();

    let active_model = ActiveModel {
        user_id: Set(user_id),
        token_hash: Set(token_hash),
        expires_at: Set(expires_at),
        created_at: Set(now.into()),
        purpose: Set(purpose),
        ..Default::default()
    };

    Ok(active_model.insert(db).await?)
}

/// Look up a magic link token by its SHA-256 hash, scoped to the given purpose.
///
/// Purpose scoping prevents a leaked Setup token from being redeemed at the
/// password-reset endpoint, and vice versa.
pub async fn find_by_token_hash(
    db: &impl ConnectionTrait,
    token_hash: &str,
    purpose: TokenPurpose,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::TokenHash.eq(token_hash))
        .filter(Column::Purpose.eq(purpose))
        .one(db)
        .await?)
}

/// Fetch tokens of the given purpose for each of the given user IDs
/// (at most one per (user_id, purpose) pair in practice).
pub async fn find_by_user_ids(
    db: &impl ConnectionTrait,
    user_ids: &[Id],
    purpose: TokenPurpose,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.is_in(user_ids.to_vec()))
        .filter(Column::Purpose.eq(purpose))
        .all(db)
        .await?)
}

/// Find the most recent token of the given purpose for a user, if any.
///
/// Used by the rate-limit check on password-reset requests.
pub async fn find_most_recent_for_user(
    db: &impl ConnectionTrait,
    user_id: Id,
    purpose: TokenPurpose,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::Purpose.eq(purpose))
        .order_by_desc(Column::CreatedAt)
        .one(db)
        .await?)
}

/// Count tokens of the given purpose issued for a user since `since`.
///
/// Used by the rate-limit check on password-reset requests
/// (e.g. "max 5 password-reset tokens issued in the last 24h").
pub async fn count_for_user_since(
    db: &impl ConnectionTrait,
    user_id: Id,
    purpose: TokenPurpose,
    since: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::Purpose.eq(purpose))
        .filter(Column::CreatedAt.gte(since))
        .count(db)
        .await?)
}

/// Delete tokens of the given purpose for a user.
///
/// Purpose-scoped so issuing a reset token does not invalidate a pending
/// Setup token for the same user (and vice versa).
pub async fn delete_all_for_user(
    db: &impl ConnectionTrait,
    user_id: Id,
    purpose: TokenPurpose,
) -> Result<(), Error> {
    Entity::delete_many()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::Purpose.eq(purpose))
        .exec(db)
        .await?;
    Ok(())
}
