use sea_orm::Order;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use domain::{action, actions, status::Status, Id, QuerySort};

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

/// Identifies the assignee by their role within a coaching relationship,
/// or by a specific user ID.
///
/// Deserialized from the `assignee` query parameter:
/// - `"coach"` / `"coachee"` (case-insensitive) → role-based filter
/// - A valid UUID string → specific user filter
#[derive(Clone, Debug)]
pub(crate) enum AssigneeScope {
    Coach,
    Coachee,
    User(Id),
}

impl<'de> Deserialize<'de> for AssigneeScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "coach" => Ok(Self::Coach),
            "coachee" => Ok(Self::Coachee),
            other => Id::parse_str(other)
                .map(Self::User)
                .map_err(serde::de::Error::custom),
        }
    }
}

impl From<AssigneeScope> for action::AssigneeScope {
    fn from(scope: AssigneeScope) -> Self {
        match scope {
            AssigneeScope::Coach => Self::Coach,
            AssigneeScope::Coachee => Self::Coachee,
            AssigneeScope::User(id) => Self::User(id),
        }
    }
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

/// Query parameters shared by coaching relationship action endpoints:
/// - `GET /organizations/{org_id}/coaching_relationships/{rel_id}/actions`
/// - `GET /organizations/{org_id}/coaching_relationships/actions`
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// Optional: filter by assignment status. `all` (default) returns
    /// everything visible to the caller; `assigned` returns only actions
    /// with ≥1 assignee; `unassigned` returns only actions with 0 assignees.
    /// Orthogonal to `assignee` (which scopes to a specific party).
    #[serde(default)]
    pub(crate) assignee_filter: AssigneeFilter,
    /// Optional: filter by action lifecycle status (PascalCase: `NotStarted`,
    /// `InProgress`, `Completed`, `OnHold`, `WontDo`).
    pub(crate) status: Option<Status>,
    /// Optional: filter actions by assignee with **strict-contains** semantics
    /// (the action's assignees must contain the resolved user id; unassigned
    /// actions are excluded whenever this param is present). Accepts three
    /// forms: `coach` / `coachee` (case-insensitive role strings that resolve
    /// per-relationship to the relationship's `coach_id`/`coachee_id`), or a
    /// UUID string for a specific user. **Omit this param for the broad view**
    /// — visibility narrowing alone determines what each caller can see, and
    /// omitting is the correct shape for the coach's "All" tab and for any
    /// coachee caller's own page. Coachee callers may only pass `coachee` or
    /// their own UUID; other values return 403 `forbidden_assignee_scope`.
    pub(crate) assignee: Option<AssigneeScope>,
    /// Optional: field to sort by. Defaults to `due_by` when any sort param
    /// is provided.
    pub(crate) sort_by: Option<SortField>,
    /// Optional: sort direction (`asc` / `desc`). Defaults to `asc` when any
    /// sort param is provided.
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

    /// Extracts the assignee scope before the params are consumed by `into_query_params`.
    pub(crate) fn assignee_scope(&self) -> Option<action::AssigneeScope> {
        self.assignee.clone().map(Into::into)
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
            assignee_user_id: None,
            caller_visibility: action::CallerVisibility::default(),
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

    #[test]
    fn assignee_scope_deserializes_coach() {
        let json = r#"{"assignee": "coach"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        assert!(matches!(params.assignee, Some(AssigneeScope::Coach)));
    }

    #[test]
    fn assignee_scope_deserializes_coachee_case_insensitive() {
        let json = r#"{"assignee": "Coachee"}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        assert!(matches!(params.assignee, Some(AssigneeScope::Coachee)));
    }

    #[test]
    fn assignee_scope_deserializes_uuid() {
        let id = Id::new_v4();
        let json = format!(r#"{{"assignee": "{}"}}"#, id);
        let params: IndexParams = serde_json::from_str(&json).unwrap();
        assert!(matches!(params.assignee, Some(AssigneeScope::User(uid)) if uid == id));
    }

    #[test]
    fn assignee_scope_defaults_to_none() {
        let json = r#"{}"#;
        let params: IndexParams = serde_json::from_str(json).unwrap();
        assert!(params.assignee.is_none());
    }
}
