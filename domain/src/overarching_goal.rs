use crate::error::Error;
use crate::overarching_goals::Model;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{overarching_goals, query};
use sea_orm::DatabaseConnection;

pub use entity_api::overarching_goal::{create, find_by_id, update, update_status};

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<overarching_goals::Column>,
{
    let overarching_goals =
        query::find_by::<overarching_goals::Entity, overarching_goals::Column, P>(db, params)
            .await?;
    Ok(overarching_goals)
}
