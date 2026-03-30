use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use log::*;
use rand::RngCore;
use sea_orm::{ActiveModelTrait, DatabaseConnection, IntoActiveModel, Set};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::{users, Id};
use entity_api::user::generate_hash;
use service::config::Config;

/// Profile fields that can be set during magic link account setup.
#[derive(Debug, Deserialize)]
pub struct SetupProfile {
    pub display_name: Option<String>,
    pub github_username: Option<String>,
    pub github_profile_url: Option<String>,
    pub timezone: Option<String>,
}

/// Generate a magic link token for a user.
///
/// Returns the raw token string (URL-safe base64) which should be included
/// in the email URL. Only the SHA-256 hash is stored in the database.
pub async fn create_magic_link(
    db: &DatabaseConnection,
    user_id: Id,
    config: &Config,
) -> Result<String, Error> {
    // Generate 32 bytes of cryptographic randomness
    let mut raw_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw_bytes);

    // URL-safe base64 encode for the email link
    let raw_token = URL_SAFE_NO_PAD.encode(raw_bytes);

    // SHA-256 hash for storage
    let token_hash = hash_token(&raw_token);

    // Calculate expiry
    let expiry_seconds = config.magic_link_expiry_seconds() as i64;
    let expires_at = Utc::now() + Duration::seconds(expiry_seconds);

    // Delete any existing tokens for this user (one active invite at a time)
    entity_api::magic_link_token::delete_all_for_user(db, user_id).await?;

    // Insert the new token
    entity_api::magic_link_token::create(db, user_id, token_hash, expires_at.into()).await?;

    info!("Magic link token created for user {user_id}");
    Ok(raw_token)
}

/// Validate a raw magic link token.
///
/// Hashes the token, looks it up in the database, checks expiry,
/// and returns the associated user if valid.
pub async fn validate_token(
    db: &DatabaseConnection,
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

    // Check expiry
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
/// Validates the token, deletes all tokens for the user, hashes the password,
/// and persists all changes. Returns the updated user.
pub async fn complete_setup(
    db: &DatabaseConnection,
    raw_token: &str,
    password: String,
    confirm_password: String,
    profile: SetupProfile,
) -> Result<users::Model, Error> {
    if password != confirm_password {
        warn!("Password confirmation does not match during magic link setup");
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Password confirmation does not match".to_string(),
            )),
        });
    }

    // Validate and consume the token
    let user = validate_token(db, raw_token).await?;
    entity_api::magic_link_token::delete_all_for_user(db, user.id).await?;

    // Hash and set the password, apply optional profile fields
    let password_hash = generate_hash(password);
    let mut active_model = user.into_active_model();

    active_model.password = Set(Some(password_hash));

    if let Some(display_name) = profile.display_name {
        active_model.display_name = Set(Some(display_name));
    }
    if let Some(github_username) = profile.github_username {
        active_model.github_username = Set(Some(github_username));
    }
    if let Some(github_profile_url) = profile.github_profile_url {
        active_model.github_profile_url = Set(Some(github_profile_url));
    }
    if let Some(timezone) = profile.timezone {
        active_model.timezone = Set(timezone);
    }

    let updated_user = active_model
        .update(db)
        .await
        .map_err(entity_api::error::Error::from)?;

    info!("User {} completed magic link setup", updated_user.id);
    Ok(updated_user)
}

/// Compute the SHA-256 hex digest of a raw token string.
fn hash_token(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    hex::encode(hasher.finalize())
}
