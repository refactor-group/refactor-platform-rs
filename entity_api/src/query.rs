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

/// `QuerySort` is a trait that provides optional sorting capabilities for query parameters.
/// This trait works alongside `IntoQueryFilterMap` to provide a unified interface for both
/// filtering and sorting in database queries.
///
/// The default implementation returns `None` for both methods, making sorting optional.
/// Structs that need sorting functionality can implement this trait to specify their
/// sort column and order.
///
/// # Example
///
/// ```
/// use entity_api::query::QuerySort;
/// use sea_orm::Order;
/// use entity::actions::Column as ActionColumn;
///
/// #[derive(Debug)]
/// struct ActionParams {
///     sort_by: Option<ActionColumn>,
///     sort_order: Option<Order>,
/// }
///
/// impl QuerySort<ActionColumn> for ActionParams {
///     fn get_sort_column(&self) -> Option<ActionColumn> {
///         self.sort_by
///     }
///
///     fn get_sort_order(&self) -> Option<Order> {
///         self.sort_order.clone()
///     }
/// }
/// ```
pub trait QuerySort<C: ColumnTrait> {
    /// Returns the column to sort by, if any
    fn get_sort_column(&self) -> Option<C>;

    /// Returns the sort order, if any
    fn get_sort_order(&self) -> Option<Order>;
}

/// Wrapper struct that provides default QuerySort implementation for types that only need filtering
pub struct FilterOnly<T>(pub T);

impl<T, C> QuerySort<C> for FilterOnly<T>
where
    C: ColumnTrait,
{
    fn get_sort_column(&self) -> Option<C> {
        None
    }

    fn get_sort_order(&self) -> Option<Order> {
        None
    }
}

impl<T> IntoQueryFilterMap for FilterOnly<T>
where
    T: IntoQueryFilterMap,
{
    fn into_query_filter_map(self) -> QueryFilterMap {
        self.0.into_query_filter_map()
    }
}

/// Find all records of an entity by the given parameters.
///
/// This function handles both filtering (via IntoQueryFilterMap) and optional sorting
/// (via QuerySort) in a single unified interface. If the parameters don't implement
/// QuerySort or return None for sorting fields, no sorting is applied.
///
/// # Example with just filtering
/// ```no_run
/// # use entity_api::query::{find_by, FilterOnly, IntoQueryFilterMap, QueryFilterMap};
/// # use entity::actions::{Entity as ActionEntity, Column as ActionColumn};
/// # use sea_orm::{DatabaseConnection, Value};
/// #
/// # #[derive(Debug)]
/// # struct MyFilterParams {
/// #     coaching_session_id: String,
/// # }
/// #
/// # impl IntoQueryFilterMap for MyFilterParams {
/// #     fn into_query_filter_map(self) -> QueryFilterMap {
/// #         let mut map = QueryFilterMap::new();
/// #         map.insert(
/// #             "coaching_session_id".to_string(),
/// #             Some(Value::String(Some(Box::new(self.coaching_session_id)))),
/// #         );
/// #         map
/// #     }
/// # }
/// #
/// # async fn example(db: &DatabaseConnection) -> Result<(), Box<dyn std::error::Error>> {
/// let params = FilterOnly(MyFilterParams {
///     coaching_session_id: "550e8400-e29b-41d4-a716-446655440000".to_string()
/// });
/// let results = find_by::<ActionEntity, ActionColumn, _>(db, params).await?;
/// #     Ok(())
/// # }
/// ```
///
/// # Example with filtering and sorting  
/// ```no_run
/// # use entity_api::query::{find_by, IntoQueryFilterMap, QueryFilterMap, QuerySort};
/// # use entity::actions::{Entity as ActionEntity, Column as ActionColumn};
/// # use sea_orm::{DatabaseConnection, Order, Value};
/// #
/// # #[derive(Debug)]
/// # struct MyParams {
/// #     coaching_session_id: String,
/// #     sort_by: Option<ActionColumn>,
/// #     sort_order: Option<Order>,
/// # }
/// #
/// # impl IntoQueryFilterMap for MyParams {
/// #     fn into_query_filter_map(self) -> QueryFilterMap {
/// #         let mut map = QueryFilterMap::new();
/// #         map.insert(
/// #             "coaching_session_id".to_string(),
/// #             Some(Value::String(Some(Box::new(self.coaching_session_id)))),
/// #         );
/// #         map
/// #     }
/// # }
/// #
/// # impl QuerySort<ActionColumn> for MyParams {
/// #     fn get_sort_column(&self) -> Option<ActionColumn> {
/// #         self.sort_by.clone()
/// #     }
/// #     
/// #     fn get_sort_order(&self) -> Option<Order> {
/// #         self.sort_order.clone()
/// #     }
/// # }
/// #
/// # async fn example(db: &DatabaseConnection) -> Result<(), Box<dyn std::error::Error>> {
/// let params = MyParams {
///     coaching_session_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
///     sort_by: Some(ActionColumn::CreatedAt),
///     sort_order: Some(Order::Desc)
/// };
/// let results = find_by::<ActionEntity, ActionColumn, _>(db, params).await?;
/// #     Ok(())
/// # }
/// ```
pub async fn find_by<E, C, P>(db: &DatabaseConnection, params: P) -> Result<Vec<E::Model>, Error>
where
    E: EntityTrait,
    C: ColumnTrait + IntoEnumIterator,
    P: IntoQueryFilterMap + QuerySort<C>,
{
    // Extract sorting parameters before consuming params
    let sort_column = params.get_sort_column();
    let sort_order = params.get_sort_order();
    let query_filter_map = params.into_query_filter_map();

    let mut query = E::find();

    // Apply filters by iterating through the entity's defined columns
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
