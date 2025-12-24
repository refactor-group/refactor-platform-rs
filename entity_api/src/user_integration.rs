//! CRUD operations for user_integrations table.

use super::error::{EntityApiErrorKind, Error};
use entity::user_integrations::{ActiveModel, Entity, Model};
use entity::Id;
use log::*;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};

/// Creates a new user integration record
pub async fn create(db: &DatabaseConnection, user_id: Id) -> Result<Model, Error> {
    debug!("Creating new user integration for user_id: {user_id}");

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Updates an existing user integration record
pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(existing) => {
            debug!("Updating user integration: {id}");

            let active_model = ActiveModel {
                id: Unchanged(existing.id),
                user_id: Unchanged(existing.user_id),
                google_access_token: Set(model.google_access_token),
                google_refresh_token: Set(model.google_refresh_token),
                google_token_expiry: Set(model.google_token_expiry),
                google_email: Set(model.google_email),
                recall_ai_api_key: Set(model.recall_ai_api_key),
                recall_ai_region: Set(model.recall_ai_region),
                recall_ai_verified_at: Set(model.recall_ai_verified_at),
                assembly_ai_api_key: Set(model.assembly_ai_api_key),
                assembly_ai_verified_at: Set(model.assembly_ai_verified_at),
                auto_approve_ai_suggestions: Set(model.auto_approve_ai_suggestions),
                created_at: Unchanged(existing.created_at),
                updated_at: Set(chrono::Utc::now().into()),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            debug!("User integration with id {id} not found");
            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

/// Finds a user integration by ID
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Finds a user integration by user ID
pub async fn find_by_user_id(db: &DatabaseConnection, user_id: Id) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::user_integrations::Column::UserId.eq(user_id))
        .one(db)
        .await?)
}

/// Gets or creates a user integration for a user
pub async fn get_or_create(db: &DatabaseConnection, user_id: Id) -> Result<Model, Error> {
    match find_by_user_id(db, user_id).await? {
        Some(model) => Ok(model),
        None => create(db, user_id).await,
    }
}

/// Deletes a user integration by ID
pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let model = find_by_id(db, id).await?;
    Entity::delete_by_id(model.id).exec(db).await?;
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase};

    #[tokio::test]
    async fn find_by_user_id_returns_none_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_by_user_id(&db, Id::new_v4()).await?;
        assert!(result.is_none());
        Ok(())
    }
}
