//! A [`SchedulingPermit`] built without a database, for tests of the functions that take one.

use entity::roles::Role;

use crate::coaching_relationships::{self, SchedulingPermit};
use crate::{user_roles, users};

/// The proof for `relationship`'s coach, a member of its organization.
pub(crate) fn coach_permit(relationship: coaching_relationships::Model) -> SchedulingPermit {
    let coach = member_coach(&relationship);
    SchedulingPermit::verify(relationship, &coach).expect("a member coach may schedule sessions")
}

/// `relationship`'s coach, holding a role in its organization.
pub(crate) fn member_coach(relationship: &coaching_relationships::Model) -> users::Model {
    let now = chrono::Utc::now().fixed_offset();
    users::Model {
        id: relationship.coach_id,
        email: "coach@example.com".to_string(),
        first_name: "Test".to_string(),
        last_name: "Coach".to_string(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        working_hours_start: crate::working_hours::default_start(),
        working_hours_end: crate::working_hours::default_end(),
        roles: vec![user_roles::Model {
            id: crate::Id::new_v4(),
            role: Role::User,
            organization_id: Some(relationship.organization_id),
            user_id: relationship.coach_id,
            created_at: now,
            updated_at: now,
        }],
        invite_status: None,
        created_at: now,
        updated_at: now,
    }
}
