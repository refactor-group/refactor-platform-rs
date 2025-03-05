use crate::actions::Model;
use crate::error::Error;
use entity_api::query::IntoQueryFilterMap;
use entity_api::{actions, query};
use sea_orm::DatabaseConnection;

pub use entity_api::action::{create, delete_by_id, find_by_id, update, update_status};

pub async fn find_by(
    db: &DatabaseConnection,
    params: impl IntoQueryFilterMap,
) -> Result<Vec<Model>, Error> {
    let actions =
        query::find_by::<actions::Entity, actions::Column>(db, params.into_query_filter_map())
            .await?;

    Ok(actions)
}
