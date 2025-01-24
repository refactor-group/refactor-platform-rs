use entity::coaching_sessions::Model;
use entity_api::{coaching_session, error::Error};
use sea_orm::DatabaseConnection;

pub async fn create(
    db: &DatabaseConnection,
    coaching_session_model: Model,
) -> Result<Model, Error> {
    coaching_session::create(db, coaching_session_model).await
}

pub async fn find_by_id(db: &DatabaseConnection, id: entity::Id) -> Result<Option<Model>, Error> {
    coaching_session::find_by_id(db, id).await
}

pub async fn find_by_id_with_coaching_relationship(
    db: &DatabaseConnection,
    id: entity::Id,
) -> Result<(Model, entity::coaching_relationships::Model), Error> {
    coaching_session::find_by_id_with_coaching_relationship(db, id).await
}

pub async fn find_by(
    db: &DatabaseConnection,
    params: std::collections::HashMap<String, String>,
) -> Result<Vec<Model>, Error> {
    coaching_session::find_by(db, params).await
}
