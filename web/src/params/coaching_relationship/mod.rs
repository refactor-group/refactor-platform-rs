use serde::Deserialize;
use utoipa::ToSchema;

use domain::coaching_relationship::InviteeRelationship;
use domain::{
    coaching_relationship_kind::{Kind, Role},
    Id,
};

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

/// The relationship to create with the person being added to an organization.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = params::coaching_relationship::InviteeRelationshipParams)]
pub(crate) struct InviteeRelationshipParams {
    /// The other person, already a member, who takes the seat the person added does not.
    pub(crate) with_user_id: Id,
    /// What the relationship is.
    pub(crate) kind: Kind,
    /// The person added's role in this relationship, not their organization role: `coach`
    /// or `coachee` for `coaching`, `colleague` for `colleague`. Omit to make them the
    /// coachee of `coaching` or a colleague. A role the kind has no seat for is a 422
    /// with `details.reason` `role_not_in_kind`.
    pub(crate) role: Option<Role>,
}

impl From<InviteeRelationshipParams> for InviteeRelationship {
    fn from(params: InviteeRelationshipParams) -> Self {
        Self {
            with_user_id: params.with_user_id,
            kind: params.kind,
            role: params.role,
        }
    }
}
