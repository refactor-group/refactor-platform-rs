use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use entity_api::mutate;
use log::*;
use rand::RngCore;
use sea_orm::{ConnectionTrait, DatabaseConnection, IntoActiveModel, TransactionTrait, Value};
use sha2::{Digest, Sha256};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::{magic_link_tokens, users, Id};
use entity_api::user::generate_hash;
use entity_api::user_invite_status::InviteStatus;
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

/// Derive a user's invite status from their password and token state.
pub fn compute_invite_status(
    password: &Option<String>,
    token: Option<&magic_link_tokens::Model>,
) -> InviteStatus {
    match (password, token) {
        (Some(_), _) => InviteStatus::Active,
        (None, Some(t)) if t.expires_at > Utc::now() => InviteStatus::Pending,
        _ => InviteStatus::Expired,
    }
}

/// Compute the SHA-256 hex digest of a raw token string.
fn hash_token(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_token_is_deterministic() {
        let hash1 = hash_token("test_token");
        let hash2 = hash_token("test_token");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn hash_token_produces_64_char_hex() {
        let hash = hash_token("any_input");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_token_differs_for_different_inputs() {
        assert_ne!(hash_token("token_a"), hash_token("token_b"));
    }

    #[cfg(feature = "mock")]
    mod mock_tests {
        use super::*;
        use crate::error::{DomainErrorKind, EntityErrorKind, InternalErrorKind};
        use chrono::{Duration, Utc};
        use entity_api::mutate::{IntoUpdateMap, UpdateMap};
        use entity_api::{magic_link_tokens, user_roles};
        use sea_orm::prelude::DateTimeWithTimeZone;
        use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};
        use uuid::Uuid;

        /// Test-only wrapper allowing us to pass an `UpdateMap` directly to
        /// `complete_setup` (which takes `impl IntoUpdateMap`). In production,
        /// the web layer's `CompleteSetupParams` struct implements this trait.
        struct TestParams(UpdateMap);

        impl IntoUpdateMap for TestParams {
            fn into_update_map(self) -> UpdateMap {
                self.0
            }
        }

        fn setup_params(password: &str, confirm: &str, token: &str) -> TestParams {
            let mut map = UpdateMap::new();
            map.insert(
                "password".into(),
                Some(Value::String(Some(Box::new(password.into())))),
            );
            map.insert(
                "confirm_password".into(),
                Some(Value::String(Some(Box::new(confirm.into())))),
            );
            map.insert(
                "token".into(),
                Some(Value::String(Some(Box::new(token.into())))),
            );
            TestParams(map)
        }

        fn test_token_model(expires_at: DateTimeWithTimeZone) -> magic_link_tokens::Model {
            magic_link_tokens::Model {
                id: Uuid::new_v4(),
                user_id: Uuid::new_v4(),
                token_hash: hash_token("raw_token"),
                expires_at,
                created_at: Utc::now().into(),
            }
        }

        fn test_user_model(id: Uuid) -> users::Model {
            users::Model {
                id,
                email: "test@example.com".into(),
                first_name: "Test".into(),
                last_name: "User".into(),
                display_name: None,
                password: None,
                github_username: None,
                github_profile_url: None,
                timezone: "UTC".into(),
                role: Default::default(),
                roles: vec![],
                invite_status: None,
                created_at: Utc::now().into(),
                updated_at: Utc::now().into(),
            }
        }

        #[tokio::test]
        async fn validate_token_rejects_expired_token() {
            let expired_at = (Utc::now() - Duration::hours(1)).into();
            let token_model = test_token_model(expired_at);

            let db = MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![token_model]])
                .into_connection();

            let result = validate_token(&db, "raw_token").await;

            let err = result.unwrap_err();
            assert_eq!(
                err.error_kind,
                DomainErrorKind::Internal(InternalErrorKind::Entity(
                    EntityErrorKind::Unauthenticated
                ))
            );
        }

        #[tokio::test]
        async fn validate_token_rejects_unknown_token() {
            let db = MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![Vec::<magic_link_tokens::Model>::new()])
                .into_connection();

            let result = validate_token(&db, "nonexistent_token").await;

            let err = result.unwrap_err();
            assert_eq!(
                err.error_kind,
                DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound))
            );
        }

        /// Helper to build a MockDatabase with the full query sequence needed
        /// for one successful `complete_setup` call.
        fn mock_db_for_successful_setup(
            token_model: &magic_link_tokens::Model,
            user: &users::Model,
            updated_user: &users::Model,
        ) -> MockDatabase {
            MockDatabase::new(DatabaseBackend::Postgres)
                // validate_token → find_by_token_hash
                .append_query_results(vec![vec![token_model.clone()]])
                // validate_token → find_by_id (uses find_with_related)
                .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>(vec![
                    vec![(user.clone(), None)],
                ])
                // delete_all_for_user
                .append_exec_results(vec![MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }])
                // mutate::update → returns updated user
                .append_query_results(vec![vec![updated_user.clone()]])
        }

        #[tokio::test]
        async fn complete_setup_succeeds_with_matching_passwords() {
            let expires_at: DateTimeWithTimeZone = (Utc::now() + Duration::hours(1)).into();
            let token_model = test_token_model(expires_at);
            let user = test_user_model(token_model.user_id);
            let updated_user = users::Model {
                password: Some("hashed_password".into()),
                ..user.clone()
            };

            let db =
                mock_db_for_successful_setup(&token_model, &user, &updated_user).into_connection();

            let params = setup_params("my_password", "my_password", "raw_token");
            let result = complete_setup(&db, params).await;

            let returned_user = result.unwrap();
            assert_eq!(returned_user.id, updated_user.id);
            assert!(returned_user.password.is_some());
        }

        #[tokio::test]
        async fn complete_setup_rejects_mismatched_passwords() {
            let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
            let params = setup_params("password123", "different456", "dummy_token");

            let result = complete_setup(&db, params).await;

            let err = result.unwrap_err();
            match err.error_kind {
                DomainErrorKind::Validation(msg) => {
                    assert_eq!(msg, "Password confirmation does not match");
                }
                other => panic!("Expected DomainErrorKind::Validation, got {other:?}"),
            }
        }

        /// A magic link token is single-use: the first `complete_setup` call
        /// succeeds and consumes the token; a second call with the same raw
        /// token fails because `validate_token` can no longer find it.
        #[tokio::test]
        async fn complete_setup_token_is_single_use() {
            let expires_at: DateTimeWithTimeZone = (Utc::now() + Duration::hours(1)).into();
            let token_model = test_token_model(expires_at);
            let user = test_user_model(token_model.user_id);
            let updated_user = users::Model {
                password: Some("hashed_password".into()),
                ..user.clone()
            };

            let db = mock_db_for_successful_setup(&token_model, &user, &updated_user)
                // --- second call: token has been deleted ---
                // find_by_token_hash returns None
                .append_query_results(vec![Vec::<magic_link_tokens::Model>::new()])
                .into_connection();

            // First call succeeds
            let params = setup_params("my_password", "my_password", "raw_token");
            let result = complete_setup(&db, params).await;
            assert!(result.is_ok());

            // Second call with the same token fails
            let params = setup_params("my_password", "my_password", "raw_token");
            let result = complete_setup(&db, params).await;

            let err = result.unwrap_err();
            assert_eq!(
                err.error_kind,
                DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound))
            );
        }
    }
}
