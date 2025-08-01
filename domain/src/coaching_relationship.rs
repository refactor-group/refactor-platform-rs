use crate::coaching_relationships::Model;
use crate::error::Error;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{coaching_relationships, query};
use sea_orm::DatabaseConnection;

pub use entity_api::coaching_relationship::{
    create, find_by_id, find_by_organization_with_user_names, find_by_user,
    get_relationship_with_user_names, CoachingRelationshipWithUserNames,
};

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<coaching_relationships::Column>,
{
    let coaching_relationships =
        query::find_by::<coaching_relationships::Entity, coaching_relationships::Column, P>(
            db, params,
        )
        .await?;
    Ok(coaching_relationships)
}
