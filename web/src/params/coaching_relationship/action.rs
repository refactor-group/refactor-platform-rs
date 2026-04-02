use sea_orm::Order;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use domain::{action, actions, status::Status, QuerySort};

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

/// Sortable fields for coaching relationship actions endpoints.
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

/// Query parameters shared by both coaching relationship action endpoints:
/// - `GET /organizations/{org_id}/coaching_relationships/{rel_id}/actions`
/// - `GET /organizations/{org_id}/coaching_relationships/coachee-actions`
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
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
    /// Applies default sorting parameters if any sort parameter is provided.
    ///
    /// Uses `DueBy` as the default sort field for actions.
    pub(crate) fn apply_defaults(mut self) -> Self {
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

impl From<AssigneeFilter> for action::AssigneeFilter {
    fn from(filter: AssigneeFilter) -> Self {
        match filter {
            AssigneeFilter::All => Self::All,
            AssigneeFilter::Assigned => Self::Assigned,
            AssigneeFilter::Unassigned => Self::Unassigned,
        }
    }
}

impl IndexParams {
    /// Converts web-layer query params into domain-layer query params,
    /// applying sort defaults in the process.
    pub(crate) fn into_query_params(self) -> action::FindByRelationshipParams {
        let params = self.apply_defaults();
        let sort_column = params.get_sort_column();
        let sort_order = params.get_sort_order();
        action::FindByRelationshipParams {
            status: params.status,
            assignee_filter: params.assignee_filter.into(),
            sort_column,
            sort_order,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_assignee_filter_is_all() {
        let json = r#"{}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();

        assert!(matches!(params.assignee_filter, AssigneeFilter::All));
    }

    #[test]
    fn apply_defaults_sets_due_by_asc_when_sort_order_provided() {
        let json = r#"{"sort_order": "asc"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let params = params.apply_defaults();

        assert!(matches!(params.sort_by, Some(SortField::DueBy)));
        assert!(matches!(params.sort_order, Some(SortOrder::Asc)));
    }

    #[test]
    fn apply_defaults_sets_asc_when_sort_by_provided() {
        let json = r#"{"sort_by": "created_at"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let params = params.apply_defaults();

        assert!(matches!(params.sort_by, Some(SortField::CreatedAt)));
        assert!(matches!(params.sort_order, Some(SortOrder::Asc)));
    }

    #[test]
    fn no_defaults_applied_when_no_sort_params() {
        let json = r#"{}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let params = params.apply_defaults();

        assert!(params.sort_by.is_none());
        assert!(params.sort_order.is_none());
    }

    #[test]
    fn status_deserializes_pascal_case() {
        let json = r#"{"status": "InProgress"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();

        assert!(matches!(params.status, Some(Status::InProgress)));
    }

    #[test]
    fn assignee_filter_deserializes_variants() {
        let json = r#"{"assignee_filter": "assigned"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        assert!(matches!(params.assignee_filter, AssigneeFilter::Assigned));

        let json = r#"{"assignee_filter": "unassigned"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        assert!(matches!(params.assignee_filter, AssigneeFilter::Unassigned));
    }
}
