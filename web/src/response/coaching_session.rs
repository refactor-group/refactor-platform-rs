//! Enriched coaching session response DTOs
//!
//! These DTOs support optional inclusion of related resources to avoid N+1 queries.
//! Related data is only included when explicitly requested via the `include` parameter.

use domain::agreements::Model as AgreementModel;
use domain::coaching_relationships::Model as CoachingRelationshipModel;
use domain::coaching_sessions::Model as CoachingSessionModel;
use domain::organizations::Model as OrganizationModel;
use domain::overarching_goals::Model as OverarchingGoalModel;
use domain::users::Model as UserModel;
use serde::Serialize;
use utoipa::ToSchema;

/// Enriched coaching session with optional related resources
/// Reuses existing domain models to avoid duplication
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnrichedCoachingSession {
    /// The coaching session itself
    #[serde(flatten)]
    pub session: CoachingSessionModel,

    /// Coaching relationship (only if ?include=relationship)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<RelationshipWithUsers>,

    /// Organization details (only if ?include=organization)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<OrganizationModel>,

    /// Overarching goal for this session (only if ?include=goal)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overarching_goal: Option<OverarchingGoalModel>,

    /// Agreement for this session (only if ?include=agreements)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agreement: Option<AgreementModel>,
}

/// Relationship with coach and coachee user details
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RelationshipWithUsers {
    /// The relationship itself
    #[serde(flatten)]
    pub relationship: CoachingRelationshipModel,

    /// Coach user details
    pub coach: UserModel,

    /// Coachee user details
    pub coachee: UserModel,
}

impl EnrichedCoachingSession {
    /// Create from a basic coaching session model (no includes)
    pub fn from_model(session: CoachingSessionModel) -> Self {
        Self {
            session,
            relationship: None,
            organization: None,
            overarching_goal: None,
            agreement: None,
        }
    }

    /// Add relationship data with coach and coachee users
    pub fn with_relationship(
        mut self,
        relationship: CoachingRelationshipModel,
        coach: UserModel,
        coachee: UserModel,
    ) -> Self {
        self.relationship = Some(RelationshipWithUsers {
            relationship,
            coach,
            coachee,
        });
        self
    }

    /// Add organization data
    pub fn with_organization(mut self, organization: OrganizationModel) -> Self {
        self.organization = Some(organization);
        self
    }

    /// Add overarching goal data
    pub fn with_goal(mut self, goal: OverarchingGoalModel) -> Self {
        self.overarching_goal = Some(goal);
        self
    }

    /// Add agreement data
    pub fn with_agreement(mut self, agreement: AgreementModel) -> Self {
        self.agreement = Some(agreement);
        self
    }
}
