use crate::error::Error;
use crate::overarching_goals::Model;
use entity_api::query::IntoQueryFilterMap;
use entity_api::{overarching_goals, query};
use sea_orm::DatabaseConnection;

pub use entity_api::overarching_goal::{create, find_by_id, update, update_status};

pub async fn find_by(
    db: &DatabaseConnection,
    params: impl IntoQueryFilterMap,
) -> Result<Vec<Model>, Error> {
    let overarching_goals = query::find_by::<overarching_goals::Entity, overarching_goals::Column>(
        db,
        params.into_query_filter_map(),
    )
    .await?;

    Ok(overarching_goals)
}
