use sea_orm::Order;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use domain::{actions, status::Status, Id, QuerySort};

/// Scope for user actions query.
///
/// Determines how the user is related to the actions being queried.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(example = "sessions")]
pub(crate) enum Scope {
    /// Actions assigned to this user
    #[serde(rename = "assigned")]
    Assigned,
    /// Actions from coaching sessions where user is coach or coachee (default)
    #[serde(rename = "sessions")]
    #[default]
    Sessions,
}

/// Filter for actions by assignee status.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(example = "all")]
pub(crate) enum AssigneeFilter {
    /// Return all actions regardless of assignee status (default)
    #[serde(rename = "all")]
    #[default]
    All,
    /// Return only actions that have at least one assignee
    #[serde(rename = "assigned")]
    Assigned,
    /// Return only actions that have no assignees
    #[serde(rename = "unassigned")]
    Unassigned,
}

/// Sortable fields for user actions endpoint.
///
/// Maps query parameter values (e.g., `?sort_by=due_by`) to database columns.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(example = "due_by")]
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
/// This unified endpoint supports multiple query modes via the `scope` parameter:
/// - `scope=assigned`: Actions where the user is an assignee
/// - `scope=sessions`: Actions from coaching sessions where user is coach or coachee (default)
///
/// Additional filters can be combined with the scope.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// User ID from URL path (not a query parameter)
    #[serde(skip)]
    pub(crate) user_id: Id,
    /// Scope: how the user relates to actions (default: sessions)
    #[serde(default)]
    pub(crate) scope: Scope,
    /// Optional: filter to a specific coaching session
    pub(crate) coaching_session_id: Option<Id>,
    /// Optional: filter by assignee status (all, assigned, unassigned)
    #[serde(default)]
    pub(crate) assignee_filter: AssigneeFilter,
    /// Optional: filter by action status
    pub(crate) status: Option<Status>,
    /// Optional: field to sort by
    pub(crate) sort_by: Option<SortField>,
    /// Optional: sort direction
    pub(crate) sort_order: Option<SortOrder>,
}

impl IndexParams {
    /// Sets the user_id field from the URL path parameter.
    pub fn with_user_id(mut self, user_id: Id) -> Self {
        self.user_id = user_id;
        self
    }

    /// Applies default sorting parameters if any sort parameter is provided.
    ///
    /// Uses `DueBy` as the default sort field for actions.
    pub fn apply_defaults(mut self) -> Self {
        <Self as WithSortDefaults>::apply_sort_defaults(
            &mut self.sort_by,
            &mut self.sort_order,
            SortField::DueBy,
        );
        self
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
