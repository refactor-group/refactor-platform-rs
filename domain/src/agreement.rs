use crate::agreements::Model;
use crate::error::Error;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{agreements, query};
use sea_orm::DatabaseConnection;

pub use entity_api::agreement::{create, delete_by_id, find_by_id, update};

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<agreements::Column>,
{
    let agreements =
        query::find_by::<agreements::Entity, agreements::Column, P>(db, params).await?;
    Ok(agreements)
}
