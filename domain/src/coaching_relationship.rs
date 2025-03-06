use crate::coaching_relationships::Model;
use crate::error::Error;
use entity_api::query::IntoQueryFilterMap;
use entity_api::{coaching_relationships, query};
use sea_orm::DatabaseConnection;

pub use entity_api::coaching_relationship::{
    create, find_by_id, find_by_organization_with_user_names, find_by_user,
    get_relationship_with_user_names, CoachingRelationshipWithUserNames,
};

pub async fn find_by(
    db: &DatabaseConnection,
    params: impl IntoQueryFilterMap,
) -> Result<Vec<Model>, Error> {
    let coaching_relationships = query::find_by::<
        coaching_relationships::Entity,
        coaching_relationships::Column,
    >(db, params.into_query_filter_map())
    .await?;

    Ok(coaching_relationships)
}
