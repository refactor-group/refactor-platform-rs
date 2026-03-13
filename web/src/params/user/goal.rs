use sea_orm::{Order, Value};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use domain::{goals, Id, IntoQueryFilterMap, QueryFilterMap, QuerySort};

/// Sortable fields for user goals endpoint.
///
/// Maps query parameter values (e.g., `?sort_by=title`) to database columns.
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

/// Query parameters for GET `/users/{user_id}/goals` endpoint.
///
/// Supports filtering by coaching relationship and status, plus standard sorting.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// User ID from URL path (not a query parameter)
    #[serde(skip)]
    pub(crate) user_id: Id,
    /// Optional: filter goals by coaching relationship
    pub(crate) coaching_relationship_id: Option<Id>,
    /// Optional: filter goals by status (e.g., "in_progress", "completed")
    pub(crate) status: Option<String>,
    /// Optional: field to sort by (defaults via WithSortDefaults)
    pub(crate) sort_by: Option<SortField>,
    /// Optional: sort direction (defaults via WithSortDefaults)
    pub(crate) sort_order: Option<SortOrder>,
}

impl IndexParams {
    /// Sets the user_id field (useful when user_id comes from path parameter).
    pub fn with_user_id(mut self, user_id: Id) -> Self {
        self.user_id = user_id;
        self
    }

    /// Applies default sorting parameters if any sort parameter is provided.
    pub fn apply_defaults(mut self) -> Self {
        <Self as WithSortDefaults>::apply_sort_defaults(
            &mut self.sort_by,
            &mut self.sort_order,
            SortField::Title,
        );
        self
    }
}

impl IntoQueryFilterMap for IndexParams {
    fn into_query_filter_map(self) -> QueryFilterMap {
        let mut query_filter_map = QueryFilterMap::new();

        query_filter_map.insert(
            "user_id".to_string(),
            Some(Value::Uuid(Some(Box::new(self.user_id)))),
        );

        if let Some(coaching_relationship_id) = self.coaching_relationship_id {
            query_filter_map.insert(
                "coaching_relationship_id".to_string(),
                Some(Value::Uuid(Some(Box::new(coaching_relationship_id)))),
            );
        }

        if let Some(status) = self.status {
            query_filter_map.insert(
                "status".to_string(),
                Some(Value::String(Some(Box::new(status)))),
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
