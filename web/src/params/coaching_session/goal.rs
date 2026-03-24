use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use domain::Id;

/// Request body for linking an existing goal to a coaching session.
/// The `coaching_session_id` comes from the URL path parameter.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct LinkParams {
    pub(crate) goal_id: Id,
}

/// Query parameters for the batch session-goals endpoint.
///
/// Exactly one filter is required — providing neither or both returns 400.
/// - `coaching_relationship_id`: fetch goals for all sessions in the relationship
/// - `coaching_session_ids`: fetch goals for specific sessions (comma-separated UUIDs)
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct BatchIndexParams {
    pub(crate) coaching_relationship_id: Option<Id>,
    #[serde(default, deserialize_with = "deserialize_comma_separated_ids")]
    pub(crate) coaching_session_ids: Vec<Id>,
}

impl BatchIndexParams {
    /// Validates that exactly one filter is provided.
    /// Returns `true` if valid, `false` if neither or both filters are set.
    pub(crate) fn is_valid(&self) -> bool {
        let has_relationship = self.coaching_relationship_id.is_some();
        let has_session_ids = !self.coaching_session_ids.is_empty();
        has_relationship ^ has_session_ids
    }
}

/// Deserializes a comma-separated string of UUIDs into a `Vec<Id>`.
fn deserialize_comma_separated_ids<'de, D>(deserializer: D) -> Result<Vec<Id>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;

    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        None => Ok(Vec::new()),
        Some(s) if s.is_empty() => Ok(Vec::new()),
        Some(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Id::parse_str(s).map_err(serde::de::Error::custom))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_params_valid_with_relationship_id_only() {
        let params = BatchIndexParams {
            coaching_relationship_id: Some(Id::new_v4()),
            coaching_session_ids: vec![],
        };
        assert!(params.is_valid());
    }

    #[test]
    fn batch_params_valid_with_session_ids_only() {
        let params = BatchIndexParams {
            coaching_relationship_id: None,
            coaching_session_ids: vec![Id::new_v4(), Id::new_v4()],
        };
        assert!(params.is_valid());
    }

    #[test]
    fn batch_params_invalid_with_no_filters() {
        let params = BatchIndexParams {
            coaching_relationship_id: None,
            coaching_session_ids: vec![],
        };
        assert!(!params.is_valid());
    }

    #[test]
    fn batch_params_invalid_with_both_filters() {
        let params = BatchIndexParams {
            coaching_relationship_id: Some(Id::new_v4()),
            coaching_session_ids: vec![Id::new_v4()],
        };
        assert!(!params.is_valid());
    }
}
