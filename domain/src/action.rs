use crate::actions::Model;
use crate::error::Error;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{actions, query};
use sea_orm::DatabaseConnection;

pub use entity_api::action::{create, delete_by_id, find_by_id, update, update_status};

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<actions::Column>,
{
    let actions = query::find_by::<actions::Entity, actions::Column, P>(db, params).await?;
    Ok(actions)
}
