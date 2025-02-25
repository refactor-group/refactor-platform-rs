use crate::{error::Error, QueryFilterMap};
use sea_orm::strum::IntoEnumIterator;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

pub async fn find_by<E, C>(
    db: &DatabaseConnection,
    query_filter_map: QueryFilterMap,
) -> Result<Vec<E::Model>, Error>
where
    E: EntityTrait,
    C: ColumnTrait + IntoEnumIterator,
{
    let mut query = E::find();

    for column in C::iter() {
        if let Some(value) = query_filter_map.get(&column.to_string()) {
            query = query.filter(column.eq(value));
        }
    }

    Ok(query.all(db).await?)
}
