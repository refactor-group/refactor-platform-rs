use sea_orm::{Order, Value};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use domain::{actions, status::Status, Id, IntoQueryFilterMap, QueryFilterMap, QuerySort};

/// Sortable fields for user actions endpoint.
///
/// Maps query parameter values (e.g., `?sort_by=due_by`) to database columns.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = "created_at")]
pub(crate) enum SortField {
    #[serde(rename = "due_by")]
    DueBy,
    #[serde(rename = "created_at")]
    CreatedAt,
    #[serde(rename = "updated_at")]
    UpdatedAt,
}

/// Query parameters for GET `/users/{user_id}/actions` endpoint.
///
/// Supports filtering by coaching session and status, plus standard sorting.
/// The `user_id` is populated from the URL path parameter, not query string.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// User ID from URL path (not a query parameter)
    #[serde(skip)]
    pub(crate) user_id: Id,
    /// Optional: filter actions by coaching session
    pub(crate) coaching_session_id: Option<Id>,
    /// Optional: filter actions by status (e.g., "open", "completed")
    pub(crate) status: Option<Status>,
    /// Optional: field to sort by (defaults via WithSortDefaults)
    pub(crate) sort_by: Option<SortField>,
    /// Optional: sort direction (defaults via WithSortDefaults)
    pub(crate) sort_order: Option<SortOrder>,
}

impl IndexParams {
    /// Sets the user_id field (useful when user_id comes from path parameter).
    ///
    /// This allows using `Query<IndexParams>` to deserialize query parameters,
    /// then setting the path-based user_id afterward.
    pub fn with_user_id(mut self, user_id: Id) -> Self {
        self.user_id = user_id;
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

        if let Some(coaching_session_id) = self.coaching_session_id {
            query_filter_map.insert(
                "coaching_session_id".to_string(),
                Some(Value::Uuid(Some(Box::new(coaching_session_id)))),
            );
        }

        if let Some(status) = self.status {
            query_filter_map.insert(
                "status".to_string(),
                Some(Value::String(Some(Box::new(status.to_string())))),
            );
        }

        query_filter_map
    }
}

impl QuerySort<actions::Column> for IndexParams {
    fn get_sort_column(&self) -> Option<actions::Column> {
        self.sort_by.as_ref().map(|field| match field {
            SortField::DueBy => actions::Column::DueBy,
            SortField::CreatedAt => actions::Column::CreatedAt,
            SortField::UpdatedAt => actions::Column::UpdatedAt,
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
