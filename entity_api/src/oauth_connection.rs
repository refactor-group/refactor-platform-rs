use super::error::{EntityApiErrorKind, Error};
use entity::oauth_connections::{ActiveModel, Column, Entity, Model};
use entity::provider::Provider;
use entity::Id;
use log::debug;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};

/// Creates a new OAuth connection record
pub async fn create(db: &DatabaseConnection, model: Model) -> Result<Model, Error> {
    debug!(
        "Creating OAuth connection for user_id: {}, provider: {}",
        model.user_id, model.provider
    );

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        user_id: Set(model.user_id),
        provider: Set(model.provider),
        external_account_id: Set(model.external_account_id),
        external_email: Set(model.external_email),
        access_token: Set(model.access_token),
        refresh_token: Set(model.refresh_token),
        token_expires_at: Set(model.token_expires_at),
        token_type: Set(model.token_type),
        scopes: Set(model.scopes),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Finds an OAuth connection by user ID and provider (unique pair)
pub async fn find_by_user_and_provider(
    db: &DatabaseConnection,
    user_id: Id,
    provider: Provider,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::Provider.eq(provider.to_string()))
        .one(db)
        .await?)
}

/// Updates tokens on an existing OAuth connection
pub async fn update_tokens(
    db: &DatabaseConnection,
    id: Id,
    access_token: String,
    refresh_token: Option<String>,
    token_expires_at: Option<DateTimeUtc>,
) -> Result<Model, Error> {
    let existing = Entity::find_by_id(id).one(db).await?.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })?;

    debug!("Updating OAuth connection tokens: {id}");

    let active_model = ActiveModel {
        id: Unchanged(existing.id),
        user_id: Unchanged(existing.user_id),
        provider: Unchanged(existing.provider),
        external_account_id: Unchanged(existing.external_account_id),
        external_email: Unchanged(existing.external_email),
        access_token: Set(access_token),
        refresh_token: Set(refresh_token),
        token_expires_at: Set(token_expires_at.map(|t| t.into())),
        token_type: Unchanged(existing.token_type),
        scopes: Unchanged(existing.scopes),
        created_at: Unchanged(existing.created_at),
        updated_at: Set(chrono::Utc::now().into()),
    };

    Ok(active_model.update(db).await?.try_into_model()?)
}

/// Deletes an OAuth connection by user ID and provider (disconnect)
pub async fn delete_by_user_and_provider(
    db: &DatabaseConnection,
    user_id: Id,
    provider: Provider,
) -> Result<(), Error> {
    let connection = find_by_user_and_provider(db, user_id, provider).await?;
    match connection {
        Some(model) => {
            Entity::delete_by_id(model.id).exec(db).await?;
            Ok(())
        }
        None => Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        }),
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn test_model() -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            user_id: Id::new_v4(),
            provider: Provider::Google,
            external_account_id: Some("google-123".to_string()),
            external_email: Some("test@gmail.com".to_string()),
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            token_expires_at: Some(now.into()),
            token_type: "Bearer".to_string(),
            scopes: "openid email".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_returns_a_new_oauth_connection() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = create(&db, model.clone()).await?;

        assert_eq!(result.user_id, model.user_id);
        assert_eq!(result.provider, Provider::Google);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_user_and_provider_returns_none_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_by_user_and_provider(&db, Id::new_v4(), Provider::Google).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn find_by_user_and_provider_returns_model_when_found() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = find_by_user_and_provider(&db, model.user_id, Provider::Google).await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().user_id, model.user_id);
        Ok(())
    }

    #[tokio::test]
    async fn update_tokens_updates_access_and_refresh_tokens() -> Result<(), Error> {
        let model = test_model();
        let mut updated = model.clone();
        updated.access_token = "new-access-token".to_string();
        updated.refresh_token = Some("new-refresh-token".to_string());

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // First query: find_by_id
            .append_query_results(vec![vec![model.clone()]])
            // Second query: update result
            .append_query_results(vec![vec![updated.clone()]])
            .into_connection();

        let result = update_tokens(
            &db,
            model.id,
            "new-access-token".to_string(),
            Some("new-refresh-token".to_string()),
            None,
        )
        .await?;

        assert_eq!(result.access_token, "new-access-token");
        assert_eq!(result.refresh_token, Some("new-refresh-token".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn update_tokens_returns_error_when_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = update_tokens(&db, Id::new_v4(), "token".to_string(), None, None).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_by_user_and_provider_executes_delete() -> Result<(), Error> {
        let model = test_model();
        let user_id = model.user_id;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // First query: find_by_user_and_provider
            .append_query_results(vec![vec![model.clone()]])
            // Second: delete exec result
            .append_exec_results(vec![sea_orm::MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        delete_by_user_and_provider(&db, user_id, Provider::Google).await?;
        Ok(())
    }

    #[tokio::test]
    async fn delete_by_user_and_provider_returns_error_when_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = delete_by_user_and_provider(&db, Id::new_v4(), Provider::Google).await;
        assert!(result.is_err());
    }
}
