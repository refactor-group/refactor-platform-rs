use crate::coaching_relationship_kind::{Kind, Role, Seat};
use crate::coaching_relationships::{Model, SchedulingPermit};
use crate::error::{reason, DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::Id;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::user_role::retain_organization_members;
use entity_api::{coaching_relationships, query};
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};

pub(crate) use entity_api::coaching_relationship::notify_member_ids;
pub use entity_api::coaching_relationship::{
    create, find_by_coach_and_organization, find_by_id, find_by_organization_with_user_names,
    find_by_user, find_by_user_and_organization, find_by_user_and_organization_with_user_names,
    find_by_user_id_with_user_names, find_coach_and_coachee, get_relationship_with_user_names,
    is_coach_of, CoachingRelationshipWithUserNames, Draft, Membership, RoleFilterable,
    WithMembership,
};

/// The relationship to create for a person joining an organization: with whom, and what kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteeRelationship {
    /// The other person; for coaching, the coach.
    pub with_user_id: Id,
    pub kind: Kind,
}

impl InviteeRelationship {
    /// The draft placing `with_user_id` in the coach seat and the invitee in the coachee seat.
    pub fn draft_for(self, invitee_id: Id) -> Draft {
        Draft {
            coach_id: self.with_user_id,
            coachee_id: invitee_id,
            kind: self.kind,
        }
    }
}

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<coaching_relationships::Column>,
{
    let coaching_relationships =
        query::find_by::<coaching_relationships::Entity, coaching_relationships::Column, P>(
            db, params,
        )
        .await?;
    Ok(coaching_relationships)
}

/// Finds coaching relationships for a user within an organization, respecting role-based access.
///
/// - SuperAdmins (global role with organization_id = NULL) see all relationships in the organization
/// - Admins (organization-specific role) see all relationships in their organization
/// - Regular users see only relationships where they are the coach or coachee
///
/// This function uses a database transaction to prevent Time-of-Check to Time-of-Use (TOCTOU)
/// vulnerabilities by ensuring the role check and data retrieval happen atomically within
/// a consistent database snapshot.
pub async fn find_by_organization_for_user_with_user_names(
    db: &DatabaseConnection,
    user_id: crate::Id,
    organization_id: crate::Id,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    // Begin transaction to ensure atomicity and prevent TOCTOU vulnerabilities
    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    // Check if user has admin access using entity_api layer
    let is_admin = entity_api::user::has_admin_access(&txn, user_id, organization_id)
        .await
        .map_err(|e| Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::DbTransaction,
            )),
        })?;

    let coaching_relationships = if is_admin {
        // Admin users see all relationships in the organization
        find_by_organization_with_user_names(&txn, organization_id).await?
    } else {
        // Regular users see only relationships they're associated with (as coach or coachee)
        find_by_user_and_organization_with_user_names(&txn, user_id, organization_id).await?
    };

    // Commit transaction
    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    Ok(coaching_relationships)
}

/// A 409 for the first participant, coach seat first, whose seat must still be in the
/// organization for sessions to be scheduled or changed but who is not among `member_ids`,
/// the relationship's participants still in its organization.
pub(crate) fn ensure_required_participants_are_members(
    relationship: &Model,
    member_ids: &[Id],
) -> Result<(), Error> {
    let rules = relationship.kind.rules();
    [
        (Seat::CoachId, relationship.coach_id),
        (Seat::CoacheeId, relationship.coachee_id),
    ]
    .into_iter()
    .find(|&(seat, user_id)| {
        rules.must_be_member_to_schedule(seat) && !member_ids.contains(&user_id)
    })
    .map_or(Ok(()), |(seat, _)| {
        Err(participant_not_a_member(relationship, seat))
    })
}

/// The 409 for a relationship whose participant in `seat` no longer belongs to its
/// organization, its reason named by that participant's role.
pub(crate) fn participant_not_a_member(relationship: &Model, seat: Seat) -> Error {
    let (message, reason) = match relationship.kind.rules().role(seat) {
        Role::Coachee => (
            "Your coachee is no longer a member of this organization. Re-add them to the \
             organization first.",
            reason::COACHEE_NOT_A_MEMBER,
        ),
        Role::Colleague => (
            "Your colleague is no longer a member of this organization. Re-add them to the \
             organization first.",
            reason::PARTICIPANT_NOT_A_MEMBER,
        ),
        Role::Coach => (
            "Your coach is no longer a member of this organization. Re-add them to the \
             organization first.",
            reason::PARTICIPANT_NOT_A_MEMBER,
        ),
    };
    let user_id = match seat {
        Seat::CoachId => relationship.coach_id,
        Seat::CoacheeId => relationship.coachee_id,
    };
    not_a_member(message, reason, relationship.organization_id, user_id)
}

/// Refuses (403) a permit for a different relationship than the session or series being
/// changed, so a permit for one relationship can't schedule in another.
pub(crate) fn ensure_permits(
    permit: &SchedulingPermit,
    coaching_relationship_id: Id,
) -> Result<(), Error> {
    (permit.relationship().id == coaching_relationship_id)
        .then_some(())
        .ok_or_else(Error::forbidden)
}

/// A 409 for the first of `assignee_ids`, in order, no longer in the relationship's
/// organization.
pub async fn ensure_assignees_are_members(
    db: &impl ConnectionTrait,
    relationship: &Model,
    assignee_ids: &[Id],
) -> Result<(), Error> {
    if assignee_ids.is_empty() {
        return Ok(());
    }
    let members =
        retain_organization_members(db, assignee_ids, relationship.organization_id).await?;
    assignee_ids
        .iter()
        .find(|id| !members.contains(id))
        .map_or(Ok(()), |&user_id| {
            Err(not_a_member(
                "An assignee is no longer a member of this organization; re-add them to the \
                 organization first.",
                reason::ASSIGNEE_NOT_A_MEMBER,
                relationship.organization_id,
                user_id,
            ))
        })
}

/// A 409 for a user who no longer belongs to the organization, carrying `reason`, the
/// organization, and the user.
fn not_a_member(message: &str, reason: &str, organization_id: Id, user_id: Id) -> Error {
    Error::conflict_because(
        reason,
        message,
        serde_json::json!({
            "organization_id": organization_id,
            "user_id": user_id,
        }),
    )
}

#[cfg(test)]
#[path = "coaching_relationship_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(test)]
#[path = "coaching_relationship_people_sqlite_tests.rs"]
mod people_sqlite_tests;

#[cfg(test)]
#[path = "coaching_relationship_membership_sqlite_tests.rs"]
mod membership_sqlite_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn relationship() -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            organization_id: Id::new_v4(),
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            kind: crate::coaching_relationship_kind::Kind::Coaching,
            slug: "coach-coachee".into(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    /// M8-02: absent from `member_ids` is the 409; present is `Ok`. No database.
    #[test]
    fn m8_02_ensure_required_participants_are_members_reads_only_the_member_ids() {
        let relationship = relationship();

        assert!(ensure_required_participants_are_members(
            &relationship,
            &[relationship.coachee_id]
        )
        .is_ok());

        let err = ensure_required_participants_are_members(&relationship, &[relationship.coach_id])
            .expect_err("a coachee missing from member_ids is refused");
        let DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Conflict {
            details: Some(details),
            ..
        })) = err.error_kind
        else {
            panic!("expected a conflict, got {:?}", err.error_kind);
        };
        assert_eq!(details["reason"], "coachee_not_a_member");
        assert_eq!(
            details["user_id"],
            serde_json::json!(relationship.coachee_id)
        );
    }

    /// A permit admits its own relationship and refuses any other with a 403.
    #[test]
    fn a_permit_admits_only_its_own_relationship() {
        let relationship = relationship();
        let permit = crate::test_utils::proof::coach_permit(relationship.clone());

        assert!(ensure_permits(&permit, relationship.id).is_ok());

        let err = ensure_permits(&permit, Id::new_v4())
            .expect_err("another relationship's id is refused");
        assert!(matches!(
            err.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Forbidden))
        ));
    }
}
