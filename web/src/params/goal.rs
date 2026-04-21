use sea_orm::{ActiveEnum, Order, Value};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use super::sort::SortOrder;
use super::WithSortDefaults;
use domain::{goals, status::Status, Id, IntoQueryFilterMap, QueryFilterMap, QuerySort};

/// Sortable fields for goals
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = "title")]
pub(crate) enum SortField {
    #[serde(rename = "title")]
    Title,
    #[serde(rename = "created_at")]
    CreatedAt,
    #[serde(rename = "updated_at")]
    UpdatedAt,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    pub(crate) coaching_relationship_id: Id,
    pub(crate) status: Option<Status>,
    pub(crate) sort_by: Option<SortField>,
    pub(crate) sort_order: Option<SortOrder>,
}

impl IntoQueryFilterMap for IndexParams {
    fn into_query_filter_map(self) -> QueryFilterMap {
        let mut query_filter_map = QueryFilterMap::new();
        query_filter_map.insert(
            "coaching_relationship_id".to_string(),
            Some(Value::Uuid(Some(Box::new(self.coaching_relationship_id)))),
        );

        if let Some(status) = self.status {
            // Store as the snake_case DB form so the generic `find_by` helper
            // produces a valid `WHERE status = '...'` against the PG enum column.
            query_filter_map.insert(
                "status".to_string(),
                Some(Value::String(Some(Box::new(status.to_value())))),
            );
        }

        query_filter_map
    }
}

impl QuerySort<goals::Column> for IndexParams {
    fn get_sort_column(&self) -> Option<goals::Column> {
        self.sort_by.as_ref().map(|field| match field {
            SortField::Title => goals::Column::Title,
            SortField::CreatedAt => goals::Column::CreatedAt,
            SortField::UpdatedAt => goals::Column::UpdatedAt,
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
