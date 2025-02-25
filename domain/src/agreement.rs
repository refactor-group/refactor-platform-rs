use crate::agreements::Model;
use crate::error::Error;
pub use entity_api::agreement::{create, delete_by_id, find_by_id, update};
use entity_api::IntoQueryFilterMap;
use entity_api::{agreements, query};
use sea_orm::DatabaseConnection;

pub async fn find_by(
    db: &DatabaseConnection,
    params: impl IntoQueryFilterMap,
) -> Result<Vec<Model>, Error> {
    let agreements = query::find_by::<agreements::Entity, agreements::Column>(
        db,
        params.into_query_filter_map(),
    )
    .await?;

    Ok(agreements)
}
