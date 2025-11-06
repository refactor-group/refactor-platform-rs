use sea_orm::{Order, Value};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use domain::{overarching_goals, Id, IntoQueryFilterMap, QueryFilterMap, QuerySort};

/// Sortable fields for user overarching goals endpoint.
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

/// Query parameters for GET `/users/{user_id}/overarching_goals` endpoint.
///
/// Supports filtering by coaching session and standard sorting.
/// Overarching goals are long-term objectives that span multiple coaching sessions.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// User ID from URL path (not a query parameter)
    #[serde(skip)]
    pub(crate) user_id: Id,
    /// Optional: filter goals associated with a specific coaching session
    pub(crate) coaching_session_id: Option<Id>,
    /// Optional: field to sort by (defaults via WithSortDefaults)
    pub(crate) sort_by: Option<SortField>,
    /// Optional: sort direction (defaults via WithSortDefaults)
    pub(crate) sort_order: Option<SortOrder>,
}

impl IndexParams {
    /// Creates params with only user_id set (all filters empty).
    pub fn new(user_id: Id) -> Self {
        Self {
            user_id,
            coaching_session_id: None,
            sort_by: None,
            sort_order: None,
        }
    }

    /// Builder method to add optional coaching session filter and sorting.
    ///
    /// Useful for programmatically constructing params in tests or internal code.
    pub fn with_filters(
        mut self,
        coaching_session_id: Option<Id>,
        sort_by: Option<SortField>,
        sort_order: Option<SortOrder>,
    ) -> Self {
        self.coaching_session_id = coaching_session_id;
        self.sort_by = sort_by;
        self.sort_order = sort_order;
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

        query_filter_map
    }
}

impl QuerySort<overarching_goals::Column> for IndexParams {
    fn get_sort_column(&self) -> Option<overarching_goals::Column> {
        self.sort_by.as_ref().map(|field| match field {
            SortField::Title => overarching_goals::Column::Title,
            SortField::CreatedAt => overarching_goals::Column::CreatedAt,
            SortField::UpdatedAt => overarching_goals::Column::UpdatedAt,
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
