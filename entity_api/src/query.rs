use crate::{error::Error, QueryFilterMap};
use sea_orm::strum::IntoEnumIterator;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

/// Find all records of an entity by the given query filter map.
pub async fn find_by<E, C>(
    db: &DatabaseConnection,
    query_filter_map: QueryFilterMap,
) -> Result<Vec<E::Model>, Error>
where
    E: EntityTrait,
    C: ColumnTrait + IntoEnumIterator,
{
    let mut query = E::find();

    // We iterate through the entity's defined columns so that we only attempt
    // to filter by columns that exist.
    for column in C::iter() {
        if let Some(value) = query_filter_map.get(&column.to_string()) {
            query = query.filter(column.eq(value));
        }
    }

    Ok(query.all(db).await?)
}
