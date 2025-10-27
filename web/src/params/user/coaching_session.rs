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
        Some(s) => {
            let mut includes = Vec::new();
            for part in s.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    let include: IncludeParam = serde_json::from_value(
                        serde_json::Value::String(trimmed.to_string())
                    ).map_err(serde::de::Error::custom)?;
                    includes.push(include);
                }
            }
            Ok(includes)
        }
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

    /// Validates that include parameters are meaningful
    /// Returns error message if validation fails
    pub fn validate_includes(&self) -> Result<(), &'static str> {
        // organization requires relationship (can't get org without relationship)
        if self.include.contains(&IncludeParam::Organization)
            && !self.include.contains(&IncludeParam::Relationship) {
            return Err("Cannot include 'organization' without 'relationship'");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_includes_allows_organization_with_relationship() {
        let params = IndexParams {
            user_id: Id::new_v4(),
            from_date: None,
            to_date: None,
            sort_by: None,
            sort_order: None,
            include: vec![IncludeParam::Relationship, IncludeParam::Organization],
        };

        assert!(params.validate_includes().is_ok());
    }

    #[test]
    fn validate_includes_rejects_organization_without_relationship() {
        let params = IndexParams {
            user_id: Id::new_v4(),
            from_date: None,
            to_date: None,
            sort_by: None,
            sort_order: None,
            include: vec![IncludeParam::Organization],
        };

        assert!(params.validate_includes().is_err());
        assert_eq!(
            params.validate_includes().unwrap_err(),
            "Cannot include 'organization' without 'relationship'"
        );
    }

    #[test]
    fn validate_includes_allows_goal_alone() {
        let params = IndexParams {
            user_id: Id::new_v4(),
            from_date: None,
            to_date: None,
            sort_by: None,
            sort_order: None,
            include: vec![IncludeParam::Goal],
        };

        assert!(params.validate_includes().is_ok());
    }

    #[test]
    fn validate_includes_allows_agreements_alone() {
        let params = IndexParams {
            user_id: Id::new_v4(),
            from_date: None,
            to_date: None,
            sort_by: None,
            sort_order: None,
            include: vec![IncludeParam::Agreements],
        };

        assert!(params.validate_includes().is_ok());
    }

    #[test]
    fn validate_includes_allows_all_includes() {
        let params = IndexParams {
            user_id: Id::new_v4(),
            from_date: None,
            to_date: None,
            sort_by: None,
            sort_order: None,
            include: vec![
                IncludeParam::Relationship,
                IncludeParam::Organization,
                IncludeParam::Goal,
                IncludeParam::Agreements,
            ],
        };

        assert!(params.validate_includes().is_ok());
    }

    #[test]
    fn validate_includes_allows_empty_includes() {
        let params = IndexParams {
            user_id: Id::new_v4(),
            from_date: None,
            to_date: None,
            sort_by: None,
            sort_order: None,
            include: vec![],
        };

        assert!(params.validate_includes().is_ok());
    }
}
