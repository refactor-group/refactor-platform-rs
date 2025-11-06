use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::params::coaching_session::SortField;
use crate::params::sort::SortOrder;
use domain::Id;

/// Related resources that can be batch-loaded with coaching sessions.
///
/// Used in `?include=` query parameter to eliminate N+1 queries. Supports
/// comma-separated values: `?include=relationship,organization,goal,agreements`
///
/// Maps to `entity_api::coaching_session::IncludeOptions` for database queries.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum IncludeParam {
    /// Include coaching relationship (coach/coachee info)
    Relationship,
    /// Include organization (requires relationship)
    Organization,
    /// Include overarching goal
    Goal,
    /// Include session agreements
    Agreements,
}

/// Query parameters for GET `/users/{user_id}/coaching_sessions` endpoint.
///
/// Supports date range filtering, sorting, and optional batch loading of related resources.
/// The enhanced `include` parameter enables efficient data fetching (see `IncludeParam`).
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// User ID from URL path (not a query parameter)
    #[serde(skip)]
    #[allow(dead_code)]
    pub(crate) user_id: Id,
    /// Optional: filter sessions starting from this date (inclusive)
    pub(crate) from_date: Option<NaiveDate>,
    /// Optional: filter sessions up to this date (inclusive)
    pub(crate) to_date: Option<NaiveDate>,
    /// Optional: field to sort by (e.g., "date", "created_at")
    pub(crate) sort_by: Option<SortField>,
    /// Optional: sort direction (asc/desc)
    pub(crate) sort_order: Option<SortOrder>,
    /// Optional: comma-separated list of related resources to batch-load
    ///
    /// Example: `?include=relationship,organization,goal`
    ///
    /// See `IncludeParam` for valid values and N+1 query optimization details.
    #[serde(default, deserialize_with = "deserialize_comma_separated")]
    pub(crate) include: Vec<IncludeParam>,
}

/// Custom deserializer for comma-separated include parameter
fn deserialize_comma_separated<'de, D>(deserializer: D) -> Result<Vec<IncludeParam>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        None => Ok(Vec::new()),
        Some(s) if s.is_empty() => Ok(Vec::new()),
        Some(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                serde_json::from_value(serde_json::Value::String(s.to_string()))
                    .map_err(serde::de::Error::custom)
            })
            .collect(),
    }
}

impl IndexParams {
    /// Creates params with only user_id set (all filters empty, no includes).
    #[allow(dead_code)]
    pub fn new(user_id: Id) -> Self {
        Self {
            user_id,
            from_date: None,
            to_date: None,
            sort_by: None,
            sort_order: None,
            include: Vec::new(),
        }
    }

    /// Builder method to add date range filtering and sorting.
    ///
    /// Note: Does not set `include` - use field access to add related resources.
    #[allow(dead_code)]
    pub fn with_filters(
        mut self,
        from_date: Option<NaiveDate>,
        to_date: Option<NaiveDate>,
        sort_by: Option<SortField>,
        sort_order: Option<SortOrder>,
    ) -> Self {
        self.from_date = from_date;
        self.to_date = to_date;
        self.sort_by = sort_by;
        self.sort_order = sort_order;
        self
    }
}

