use super::error::Error;

use chrono::Utc;
use entity::pat_status::PATStatus;
use entity::personal_access_tokens::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::{entity::prelude::*, ConnectionTrait, Set};

/// Insert a new personal access token row.
pub async fn create(
    db: &impl ConnectionTrait,
    user_id: Id,
    token_hash: String,
) -> Result<Model, Error> {
    let now = Utc::now();

    let active_model = ActiveModel {
        user_id: Set(user_id),
        token_hash: Set(token_hash),
        status: Set(PATStatus::Active),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.insert(db).await?)
}

/// Look up a personal access token by its SHA-256 hash.
pub async fn find_by_token_hash(
    db: &impl ConnectionTrait,
    token_hash: &str,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::TokenHash.eq(token_hash))
        .one(db)
        .await?)
}

/// Find the active personal access token for a user (at most one due to partial unique index).
pub async fn find_active_by_user(
    db: &impl ConnectionTrait,
    user_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::Status.eq(PATStatus::Active))
        .one(db)
        .await?)
}

/// Deactivate a personal access token by setting its status to inactive.
pub async fn deactivate(db: &impl ConnectionTrait, pat_id: Id) -> Result<Model, Error> {
    let now = Utc::now();

    let active_model = ActiveModel {
        id: Set(pat_id),
        status: Set(PATStatus::Inactive),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.update(db).await?)
}

/// Update the `last_used_at` timestamp for a personal access token.
pub async fn touch_last_used(db: &impl ConnectionTrait, pat_id: Id) -> Result<Model, Error> {
    let now = Utc::now();

    let active_model = ActiveModel {
        id: Set(pat_id),
        last_used_at: Set(Some(now.into())),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.update(db).await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::pat_status::PATStatus;
    use entity::personal_access_tokens;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn test_pat_model(user_id: Id, status: PATStatus) -> personal_access_tokens::Model {
        let now = Utc::now();
        personal_access_tokens::Model {
            id: Id::new_v4(),
            user_id,
            token_hash: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
                .to_string(),
            status,
            last_used_at: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_inserts_active_pat() {
        let user_id = Id::new_v4();
        let expected = test_pat_model(user_id, PATStatus::Active);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        let result = create(&db, user_id, expected.token_hash.clone()).await;
        assert!(result.is_ok());

        let pat = result.unwrap();
        assert_eq!(pat.user_id, user_id);
        assert_eq!(pat.status, PATStatus::Active);
    }

    #[tokio::test]
    async fn find_by_token_hash_returns_matching_pat() {
        let user_id = Id::new_v4();
        let expected = test_pat_model(user_id, PATStatus::Active);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        let result = find_by_token_hash(&db, &expected.token_hash).await;
        assert!(result.is_ok());

        let pat = result.unwrap();
        assert!(pat.is_some());
        assert_eq!(pat.unwrap().token_hash, expected.token_hash);
    }

    #[tokio::test]
    async fn find_by_token_hash_returns_none_for_unknown() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<personal_access_tokens::Model>::new()])
            .into_connection();

        let result = find_by_token_hash(&db, "nonexistent_hash").await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_active_by_user_returns_active_pat() {
        let user_id = Id::new_v4();
        let expected = test_pat_model(user_id, PATStatus::Active);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        let result = find_active_by_user(&db, user_id).await;
        assert!(result.is_ok());

        let pat = result.unwrap();
        assert!(pat.is_some());
        assert_eq!(pat.unwrap().status, PATStatus::Active);
    }

    #[tokio::test]
    async fn find_active_by_user_returns_none_when_no_active() {
        let user_id = Id::new_v4();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<personal_access_tokens::Model>::new()])
            .into_connection();

        let result = find_active_by_user(&db, user_id).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn deactivate_sets_status_to_inactive() {
        let user_id = Id::new_v4();
        let mut expected = test_pat_model(user_id, PATStatus::Inactive);
        expected.status = PATStatus::Inactive;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        let result = deactivate(&db, expected.id).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().status, PATStatus::Inactive);
    }
}
