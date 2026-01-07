use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use domain::Id;

/// Filter for coaching relationships by user's role.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(example = "all")]
pub(crate) enum RoleFilter {
    /// Return all relationships where user is coach or coachee (default)
    #[serde(rename = "all")]
    #[default]
    All,
    /// Return only relationships where user is the coach
    #[serde(rename = "coach")]
    Coach,
    /// Return only relationships where user is the coachee
    #[serde(rename = "coachee")]
    Coachee,
}

/// Query parameters for GET `/users/{user_id}/coaching-relationships` endpoint.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// User ID from URL path (not a query parameter)
    #[serde(skip)]
    pub(crate) user_id: Id,
    /// Filter by role: all, coach, or coachee (default: all)
    #[serde(default)]
    pub(crate) role: RoleFilter,
}

impl IndexParams {
    /// Sets the user_id field from the URL path parameter.
    pub fn with_user_id(mut self, user_id: Id) -> Self {
        self.user_id = user_id;
        self
    }
}
