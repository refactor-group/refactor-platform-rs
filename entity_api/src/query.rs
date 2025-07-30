use crate::error::Error;
use sea_orm::strum::IntoEnumIterator;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, Value,
};
use std::collections::HashMap;

/// `QueryFilterMap` is a data structure that serves as a bridge for translating filter parameters
/// between different layers of the application. It is essentially a wrapper around a `HashMap`
/// where the keys are filter parameter names (as `String`) and the values are optional `Value` types
/// from `sea_orm`.
///
/// This structure is particularly useful in scenarios where you need to pass filter parameters
/// from a web request down to the database query layer in a type-safe and organized manner.
///
/// # Example
///
/// ```
/// use sea_orm::Value;
/// use entity_api::query::QueryFilterMap;
///
/// let mut query_filter_map = QueryFilterMap::new();
/// query_filter_map.insert("coaching_session_id".to_string(), Some(Value::String(Some(Box::new("a_coaching_session_id".to_string())))));
/// let filter_value = query_filter_map.get("coaching_session_id");
/// ```
pub struct QueryFilterMap {
    map: HashMap<String, Option<Value>>,
}

impl QueryFilterMap {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        // HashMap.get returns an Option and so we need to "flatten" this to a single Option
        self.map
            .get(key)
            .and_then(|inner_option| inner_option.clone())
    }

    pub fn insert(&mut self, key: String, value: Option<Value>) {
        self.map.insert(key, value);
    }
}

impl Default for QueryFilterMap {
    fn default() -> Self {
        Self::new()
    }
}

/// `IntoQueryFilterMap` is a trait that provides a method for converting a struct into a `QueryFilterMap`.
/// This is particularly useful for translating data between different layers of the application,
/// such as from web request parameters to database query filters.
///
/// Implementing this trait for a struct allows you to define how the fields of the struct should be
/// mapped to the keys and values of the `QueryFilterMap`. This ensures that the data is passed
/// in a type-safe and organized manner.
///
/// # Example
///
/// ```
/// use entity_api::query::QueryFilterMap;
/// use entity_api::query::IntoQueryFilterMap;
///
/// #[derive(Debug)]
/// struct MyParams {
///     coaching_session_id: String,
/// }
///
/// impl IntoQueryFilterMap for MyParams {
///     fn into_query_filter_map(self) -> QueryFilterMap {
///         let mut query_filter_map = QueryFilterMap::new();
///         query_filter_map.insert(
///             "coaching_session_id".to_string(),
///             Some(sea_orm::Value::String(Some(Box::new(self.coaching_session_id)))),
///         );
///         query_filter_map
///     }
/// }
/// ```
pub trait IntoQueryFilterMap {
    fn into_query_filter_map(self) -> QueryFilterMap;
}

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

/// Find all records of an entity by the given query filter map with optional sorting.
pub async fn find_by_with_sort<E, C>(
    db: &DatabaseConnection,
    query_filter_map: QueryFilterMap,
    sort_column: Option<C>,
    sort_order: Option<Order>,
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

    // Apply sorting if both column and order are provided
    if let (Some(column), Some(order)) = (sort_column, sort_order) {
        query = query.order_by(column, order);
    }

    Ok(query.all(db).await?)
}
