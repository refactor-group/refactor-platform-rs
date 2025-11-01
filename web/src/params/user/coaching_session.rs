use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::params::coaching_session::SortField;
use crate::params::sort::SortOrder;
use domain::Id;

/// Include parameter for optionally fetching related resources
/// Supports comma-separated values: ?include=relationship,organization,goal,agreements
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum IncludeParam {
    Relationship,
    Organization,
    Goal,
    Agreements,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    #[serde(skip)]
    #[allow(dead_code)]
    pub(crate) user_id: Id,
    pub(crate) from_date: Option<NaiveDate>,
    pub(crate) to_date: Option<NaiveDate>,
    pub(crate) sort_by: Option<SortField>,
    pub(crate) sort_order: Option<SortOrder>,
    /// Comma-separated list of related resources to include
    /// Valid values: relationship, organization, goal, agreements
    /// Example: ?include=relationship,organization,goal
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

