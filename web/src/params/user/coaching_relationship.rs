use domain::{coaching_relationship_kind::Role, Id};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

/// Which of the user's relationships to list, by the role they hold in each.
#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(example = "all")]
pub(crate) enum RoleFilter {
    #[default]
    All,
    Coach,
    Coachee,
    Colleague,
}

impl RoleFilter {
    /// The role asked for, or `None` for every relationship.
    pub(crate) fn role(self) -> Option<Role> {
        match self {
            RoleFilter::All => None,
            RoleFilter::Coach => Some(Role::Coach),
            RoleFilter::Coachee => Some(Role::Coachee),
            RoleFilter::Colleague => Some(Role::Colleague),
        }
    }
}

/// Query parameters for GET `/users/{user_id}/coaching-relationships` endpoint.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// User ID from URL path (not a query parameter)
    #[serde(skip)]
    pub(crate) user_id: Id,
    /// The role the user holds in each listed relationship (default: all)
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
