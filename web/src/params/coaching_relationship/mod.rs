use serde::Deserialize;
use utoipa::ToSchema;

use domain::{coaching_relationship_kind::Kind, Id};

pub(crate) mod action;
pub(crate) mod goal_progress;

/// Request body for `POST /organizations/{organization_id}/coaching_relationships`.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = params::coaching_relationship::CreateParams)]
pub(crate) struct CreateParams {
    /// The person in the coach seat; for colleagues, either one of the pair.
    pub(crate) coach_id: Id,
    /// The person in the coachee seat; for colleagues, the other one of the pair.
    pub(crate) coachee_id: Id,
    /// What the relationship is. Omit for `coaching`.
    pub(crate) kind: Option<Kind>,
}
