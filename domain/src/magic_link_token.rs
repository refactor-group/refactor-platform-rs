use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use entity_api::mutate;
use log::*;
use rand::RngCore;
use sea_orm::{ConnectionTrait, DatabaseConnection, IntoActiveModel, TransactionTrait, Value};
use sha2::{Digest, Sha256};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::{users, Id};
use entity_api::user::generate_hash;
use service::config::Config;

/// Generate a magic link token for a user.
///
/// Returns the raw token string (URL-safe base64) which should be included
/// in the email URL. Only the SHA-256 hash is stored in the database.
pub async fn create_magic_link(
    db: &DatabaseConnection,
    user_id: Id,
    config: &Config,
) -> Result<String, Error> {
    let mut raw_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw_bytes);
    let raw_token = URL_SAFE_NO_PAD.encode(raw_bytes);
    let token_hash = hash_token(&raw_token);

    let expiry_seconds = config.magic_link_expiry_seconds() as i64;
    let expires_at = Utc::now() + Duration::seconds(expiry_seconds);

    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    entity_api::magic_link_token::delete_all_for_user(&txn, user_id).await?;
    entity_api::magic_link_token::create(&txn, user_id, token_hash, expires_at.into()).await?;

    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    info!("Magic link token created for user {user_id}");
    Ok(raw_token)
}

/// Validate a raw magic link token.
///
/// Hashes the token, looks it up in the database, checks expiry,
/// and returns the associated user if valid.
pub async fn validate_token(
    db: &impl ConnectionTrait,
    raw_token: &str,
) -> Result<users::Model, Error> {
    let token_hash = hash_token(raw_token);

    let token_record = entity_api::magic_link_token::find_by_token_hash(db, &token_hash)
        .await?
        .ok_or_else(|| {
            warn!("Magic link token not found");
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                    EntityErrorKind::NotFound,
                )),
            }
        })?;

    if Utc::now() > token_record.expires_at {
        warn!("Magic link token expired for user {}", token_record.user_id);
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::Unauthenticated,
            )),
        });
    }

    let user = entity_api::user::find_by_id(db, token_record.user_id).await?;
    Ok(user)
}

/// Consume a magic link token, set the user's password, and optionally update profile fields.
///
pub async fn complete_setup(
    db: &DatabaseConnection,
    params: impl mutate::IntoUpdateMap,
) -> Result<users::Model, Error> {
    let mut params = params.into_update_map();

    let password = params.remove("password")?;

    let confirm_password = params.remove("confirm_password")?;

    let raw_token = params.remove("token")?;

    if password != confirm_password {
        warn!("Password confirmation does not match during magic link setup");
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation(
                "Password confirmation does not match".to_string(),
            ),
        });
    }

    params.insert(
        "password".to_string(),
        Some(Value::String(Some(Box::new(generate_hash(password))))),
    );

    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    let user = validate_token(&txn, &raw_token).await?;
    entity_api::magic_link_token::delete_all_for_user(&txn, user.id).await?;

    let active_model = user.into_active_model();
    let updated_user =
        mutate::update::<users::ActiveModel, users::Column>(&txn, active_model, params).await?;

    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    info!("User {} completed magic link setup", updated_user.id);
    Ok(updated_user)
}

/// Compute the SHA-256 hex digest of a raw token string.
fn hash_token(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    hex::encode(hasher.finalize())
}
