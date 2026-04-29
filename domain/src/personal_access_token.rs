use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use log::*;
use rand::RngCore;
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};
use sha2::{Digest, Sha256};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::{pat_status::PATStatus, personal_access_tokens, users, Id};

/// Create a new personal access token for a user.
///
/// If the user already has an active PAT, it is deactivated first.
/// Returns the raw token string (shown once, never stored) and the persisted model.
pub async fn create_token(
    db: &DatabaseConnection,
    user_id: Id,
) -> Result<(String, personal_access_tokens::Model), Error> {
    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    // Deactivate any existing active PAT for this user
    if let Some(existing) =
        entity_api::personal_access_token::find_active_by_user(&txn, user_id).await?
    {
        info!(
            "Deactivating existing active PAT {} for user {user_id}",
            existing.id
        );
        entity_api::personal_access_token::deactivate(&txn, existing.id).await?;
    }

    // Generate a cryptographically secure random token
    let mut raw_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw_bytes);
    let raw_token = URL_SAFE_NO_PAD.encode(raw_bytes);
    let token_hash = hash_token(&raw_token);

    let model = entity_api::personal_access_token::create(&txn, user_id, token_hash).await?;

    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    info!("Created new PAT {} for user {user_id}", model.id);
    Ok((raw_token, model))
}

/// Validate a raw PAT string.
///
/// Hashes the input, looks up the PAT by hash, checks that it is active,
/// and returns the associated user with roles populated.
pub async fn validate_token(
    db: &impl ConnectionTrait,
    raw_token: &str,
) -> Result<(users::Model, personal_access_tokens::Model), Error> {
    let token_hash = hash_token(raw_token);

    let pat = entity_api::personal_access_token::find_by_token_hash(db, &token_hash)
        .await?
        .ok_or_else(|| {
            warn!("PAT not found for provided token");
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                    EntityErrorKind::Unauthenticated,
                )),
            }
        })?;

    if pat.status != PATStatus::Active {
        warn!("PAT {} is not active (status: {})", pat.id, pat.status);
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::Unauthenticated,
            )),
        });
    }

    let user = entity_api::user::find_by_id(db, pat.user_id).await?;
    Ok((user, pat))
}

/// Find the active personal access token for a user, if one exists.
pub async fn find_active_by_user(
    db: &impl ConnectionTrait,
    user_id: Id,
) -> Result<Option<personal_access_tokens::Model>, Error> {
    let model = entity_api::personal_access_token::find_active_by_user(db, user_id).await?;
    Ok(model)
}

/// Deactivate a personal access token.
pub async fn deactivate_token(
    db: &impl ConnectionTrait,
    pat_id: Id,
) -> Result<personal_access_tokens::Model, Error> {
    let model = entity_api::personal_access_token::deactivate(db, pat_id).await?;
    info!("Deactivated PAT {pat_id}");
    Ok(model)
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
        let hash1 = hash_token("test_pat_token");
        let hash2 = hash_token("test_pat_token");
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
        use chrono::Utc;
        use entity_api::{personal_access_tokens, user_roles, users};
        use sea_orm::{DatabaseBackend, MockDatabase};
        use uuid::Uuid;

        fn test_pat_model(user_id: Uuid, status: PATStatus) -> personal_access_tokens::Model {
            let now = Utc::now();
            personal_access_tokens::Model {
                id: Uuid::new_v4(),
                user_id,
                token_hash: hash_token("raw_token"),
                status,
                last_used_at: None,
                created_at: now.into(),
                updated_at: now.into(),
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
        async fn validate_token_rejects_unknown_token() {
            let db = MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![Vec::<personal_access_tokens::Model>::new()])
                .into_connection();

            let result = validate_token(&db, "nonexistent_token").await;
            let err = result.unwrap_err();
            assert_eq!(
                err.error_kind,
                DomainErrorKind::Internal(InternalErrorKind::Entity(
                    EntityErrorKind::Unauthenticated
                ))
            );
        }

        #[tokio::test]
        async fn validate_token_rejects_inactive_token() {
            let user_id = Uuid::new_v4();
            let pat = test_pat_model(user_id, PATStatus::Inactive);

            let db = MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![pat]])
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
        async fn validate_token_returns_user_for_active_token() {
            let user_id = Uuid::new_v4();
            let pat = test_pat_model(user_id, PATStatus::Active);
            let user = test_user_model(user_id);

            let db = MockDatabase::new(DatabaseBackend::Postgres)
                // find_by_token_hash
                .append_query_results(vec![vec![pat.clone()]])
                // find_by_id (uses find_with_related)
                .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>(vec![
                    vec![(user.clone(), None)],
                ])
                .into_connection();

            let result = validate_token(&db, "raw_token").await;
            assert!(result.is_ok());

            let (returned_user, returned_pat) = result.unwrap();
            assert_eq!(returned_user.id, user_id);
            assert_eq!(returned_pat.id, pat.id);
        }
    }
}
