use sea_orm::Order;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::params::coaching_relationship::action::AssigneeScope;
use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use domain::{action, goal_progress, goals, status::Status, Id, QuerySort};

/// Upper bound on the number of goals this endpoint will return in a single
/// response when the caller supplies an explicit `limit`. Requests with a
/// larger `limit` are silently clamped. Omitting `limit` preserves the
/// unbounded default behavior.
pub(crate) const MAX_LIMIT: u32 = 100;

/// Sortable fields for relationship goal progress.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(example = "updated_at")]
#[allow(clippy::enum_variant_names)]
pub(crate) enum SortField {
    #[serde(rename = "updated_at")]
    UpdatedAt,
    #[serde(rename = "status_changed_at")]
    StatusChangedAt,
    #[serde(rename = "created_at")]
    CreatedAt,
}

/// Query parameters for
/// `GET /organizations/{org_id}/coaching_relationships/{rel_id}/goal_progress`.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// Optional: filter by goal status
    pub(crate) status: Option<Status>,
    /// Optional: field to sort by
    pub(crate) sort_by: Option<SortField>,
    /// Optional: sort direction
    pub(crate) sort_order: Option<SortOrder>,
    /// Optional: cap the number of goals returned. Values above MAX_LIMIT
    /// are silently clamped. Omit for unbounded results.
    pub(crate) limit: Option<u32>,
    /// Optional: scope action counts / next-due / completed-dates to a
    /// specific assignee (`coach`, `coachee`, or a UUID). Role values are
    /// resolved to a concrete user id by the controller.
    pub(crate) assignee: Option<AssigneeScope>,
    /// Optional: restrict to goals linked to a specific coaching session
    /// via the `coaching_sessions_goals` join table.
    pub(crate) coaching_session_id: Option<Id>,
}

impl IndexParams {
    /// Applies default sorting when either `sort_by` or `sort_order` is set.
    /// Default sort field is `updated_at` (most recently updated first when
    /// combined with the frontend-supplied `desc` order).
    pub(crate) fn apply_defaults(mut self) -> Self {
        <Self as WithSortDefaults>::apply_sort_defaults(
            &mut self.sort_by,
            &mut self.sort_order,
            SortField::UpdatedAt,
        );
        self
    }

    /// Extracts the assignee scope before the params are consumed by
    /// `into_query_params`. The controller then resolves role-based scopes
    /// (`Coach` / `Coachee`) against the coaching relationship model.
    pub(crate) fn assignee_scope(&self) -> Option<action::AssigneeScope> {
        self.assignee.clone().map(Into::into)
    }

    pub(crate) fn into_query_params(self) -> goal_progress::BatchProgressParams {
        let params = self.apply_defaults();
        let sort_column = params.get_sort_column();
        let sort_order = params.get_sort_order();
        goal_progress::BatchProgressParams {
            status: params.status,
            sort_column,
            sort_order,
            limit: params.limit.map(|n| n.min(MAX_LIMIT) as u64),
            // Role-based scopes need the relationship model to resolve; the
            // controller fills this in after calling `into_query_params`.
            assignee_user_id: None,
            coaching_session_id: params.coaching_session_id,
        }
    }
}

impl QuerySort<goals::Column> for IndexParams {
    fn get_sort_column(&self) -> Option<goals::Column> {
        self.sort_by.as_ref().map(|field| match field {
            SortField::UpdatedAt => goals::Column::UpdatedAt,
            SortField::StatusChangedAt => goals::Column::StatusChangedAt,
            SortField::CreatedAt => goals::Column::CreatedAt,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_deserializes_pascal_case() {
        let json = r#"{"status": "InProgress"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();

        assert!(matches!(params.status, Some(Status::InProgress)));
    }

    #[test]
    fn apply_defaults_fills_updated_at_asc_when_only_sort_order_provided() {
        let json = r#"{"sort_order": "asc"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let params = params.apply_defaults();

        assert!(matches!(params.sort_by, Some(SortField::UpdatedAt)));
        assert!(matches!(params.sort_order, Some(SortOrder::Asc)));
    }

    #[test]
    fn apply_defaults_fills_asc_when_only_sort_by_provided() {
        let json = r#"{"sort_by": "status_changed_at"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let params = params.apply_defaults();

        assert!(matches!(params.sort_by, Some(SortField::StatusChangedAt)));
        assert!(matches!(params.sort_order, Some(SortOrder::Asc)));
    }

    #[test]
    fn no_sort_defaults_applied_when_nothing_provided() {
        let json = r#"{}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let params = params.apply_defaults();

        assert!(params.sort_by.is_none());
        assert!(params.sort_order.is_none());
    }

    #[test]
    fn limit_deserializes_u32() {
        let json = r#"{"limit": 3}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.limit, Some(3));
    }

    #[test]
    fn into_query_params_clamps_limit_above_max() {
        let json = format!(r#"{{"limit": {}}}"#, MAX_LIMIT + 500);
        let params: IndexParams = serde_json::from_str(&json).unwrap();
        let domain = params.into_query_params();
        assert_eq!(domain.limit, Some(MAX_LIMIT as u64));
    }

    #[test]
    fn into_query_params_preserves_limit_below_max() {
        let json = r#"{"limit": 3}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let domain = params.into_query_params();
        assert_eq!(domain.limit, Some(3));
    }

    #[test]
    fn into_query_params_leaves_limit_unbounded_when_omitted() {
        let json = r#"{}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let domain = params.into_query_params();
        assert!(domain.limit.is_none());
    }

    #[test]
    fn assignee_scope_deserializes_coach() {
        let json = r#"{"assignee": "coach"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        assert!(matches!(
            params.assignee_scope(),
            Some(action::AssigneeScope::Coach)
        ));
    }

    #[test]
    fn assignee_scope_deserializes_coachee_case_insensitive() {
        let json = r#"{"assignee": "Coachee"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        assert!(matches!(
            params.assignee_scope(),
            Some(action::AssigneeScope::Coachee)
        ));
    }

    #[test]
    fn assignee_scope_deserializes_uuid() {
        let id = Id::new_v4();
        let json = format!(r#"{{"assignee": "{id}"}}"#);
        let params: IndexParams = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            params.assignee_scope(),
            Some(action::AssigneeScope::User(uid)) if uid == id
        ));
    }

    #[test]
    fn into_query_params_leaves_assignee_user_id_unresolved() {
        // Role-based scopes need the relationship model — controller fills this in.
        let json = r#"{"assignee": "coach"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        let domain = params.into_query_params();
        assert!(domain.assignee_user_id.is_none());
    }

    #[test]
    fn coaching_session_id_deserializes() {
        let id = Id::new_v4();
        let json = format!(r#"{{"coaching_session_id": "{id}"}}"#);
        let params: IndexParams = serde_json::from_str(&json).unwrap();
        assert_eq!(params.coaching_session_id, Some(id));

        let domain = params.into_query_params();
        assert_eq!(domain.coaching_session_id, Some(id));
    }
}
