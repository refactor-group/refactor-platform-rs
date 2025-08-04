use sea_orm::{Order, Value};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use super::sort::SortOrder;
use super::WithSortDefaults;
use domain::{agreements, Id, IntoQueryFilterMap, QueryFilterMap, QuerySort};

/// Sortable fields for agreements
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = "body")]
pub(crate) enum SortField {
    #[serde(rename = "body")]
    Body,
    #[serde(rename = "created_at")]
    CreatedAt,
    #[serde(rename = "updated_at")]
    UpdatedAt,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    pub(crate) coaching_session_id: Id,
    pub(crate) sort_by: Option<SortField>,
    pub(crate) sort_order: Option<SortOrder>,
}

impl IntoQueryFilterMap for IndexParams {
    fn into_query_filter_map(self) -> QueryFilterMap {
        let mut query_filter_map = QueryFilterMap::new();
        query_filter_map.insert(
            "coaching_session_id".to_string(),
            Some(Value::Uuid(Some(Box::new(self.coaching_session_id)))),
        );

        query_filter_map
    }
}

impl QuerySort<agreements::Column> for IndexParams {
    fn get_sort_column(&self) -> Option<agreements::Column> {
        self.sort_by.as_ref().map(|field| match field {
            SortField::Body => agreements::Column::Body,
            SortField::CreatedAt => agreements::Column::CreatedAt,
            SortField::UpdatedAt => agreements::Column::UpdatedAt,
        })
    }

    fn get_sort_order(&self) -> Option<Order> {
        self.sort_order.as_ref().map(|order| match order {
            SortOrder::Asc => Order::Asc,
            SortOrder::Desc => Order::Desc,
        })
    }
}

impl WithSortDefaults for IndexParams {
    type SortField = SortField;
}
