use crate::audit_event::{self, NewEvent, Recorder};
use crate::coaching_relationship_kind::{Kind, Role, Seat};
use crate::coaching_relationships::{Model, SchedulingPermit};
use crate::error::{reason, DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::search::visibility_for;
use crate::{users, Actor, Id};
use entity_api::coaching_relationship::Settled;
use entity_api::error::Error as EntityApiError;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::user_role::retain_organization_members;
use entity_api::{coaching_relationships, query};
use sea_orm::{ConnectionTrait, DatabaseConnection, DatabaseTransaction, TransactionTrait};

pub mod kind_switch;

pub(crate) use entity_api::coaching_relationship::{
    create, find_by_organization_with_user_names, find_by_user_and_organization_with_user_names,
    find_by_user_id_with_user_names, find_with_coach_and_coachee, get_relationship_with_user_names,
    notify_member_ids,
};
pub use entity_api::coaching_relationship::{
    find_by_coach_and_organization, find_by_id, find_by_user, find_by_user_and_organization,
    find_coach_and_coachee, is_coach_of, Capabilities, CoachingRelationshipWithUserNames, Draft,
    Membership, Participant, WithMembership,
};

/// The relationship to create for a person joining an organization: with whom, what kind,
/// and what the invitee is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteeRelationship {
    /// The other person, who takes the seat the invitee does not.
    pub with_user_id: Id,
    pub kind: Kind,
    /// The invitee's role in the relationship; `None` seats them as the coachee.
    pub role: Option<Role>,
}

impl InviteeRelationship {
    /// Settles which seat the invitee takes: the coachee seat when `role` is omitted or both
    /// seats hold it, else the seat holding `role`. Refuses with a 422 (`role_not_in_kind`) a
    /// role neither seat of the kind holds.
    pub fn seated(self) -> Result<SeatedInvitee, Error> {
        let rules = self.kind.rules();
        self.role
            .map_or(Some(Seat::CoacheeId), |role| {
                [Seat::CoacheeId, Seat::CoachId]
                    .into_iter()
                    .find(|&seat| rules.role(seat) == role)
            })
            .map(|invitee_seat| SeatedInvitee {
                with_user_id: self.with_user_id,
                kind: self.kind,
                invitee_seat,
            })
            .ok_or_else(|| {
                Error::invalid_because(
                    reason::ROLE_NOT_IN_KIND,
                    "The relationship's kind has no seat with the role given for the new member.",
                )
            })
    }
}

/// An invitee relationship whose invitee seat is settled, ready to draft once the invitee's
/// id is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatedInvitee {
    with_user_id: Id,
    kind: Kind,
    invitee_seat: Seat,
}

impl SeatedInvitee {
    /// The draft placing the invitee in their seat and `with_user_id` in the other.
    pub fn draft_for(self, invitee_id: Id) -> Draft {
        let (coach_id, coachee_id) = match self.invitee_seat {
            Seat::CoachId => (invitee_id, self.with_user_id),
            Seat::CoacheeId => (self.with_user_id, invitee_id),
        };
        Draft {
            coach_id,
            coachee_id,
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

/// Creates the relationship `draft` describes in the organization, or returns the pair's
/// existing one of that kind, as `caller` sees it. A new relationship is recorded in the
/// audit log with `caller` as the actor.
pub async fn create_for_caller(
    db: &DatabaseConnection,
    recorder: &dyn Recorder,
    caller: &users::Model,
    organization_id: Id,
    draft: Draft,
) -> Result<CoachingRelationshipWithUserNames, Error> {
    let txn = db.begin().await.map_err(EntityApiError::from)?;
    let settled = create(&txn, organization_id, draft).await?;
    recorder
        .record_all(
            &txn,
            creation_event(Actor::new(caller.id), &settled)
                .into_iter()
                .collect(),
        )
        .await?;
    txn.commit().await.map_err(EntityApiError::from)?;
    Ok(settled.relationship.for_caller(caller))
}

/// Creates the invitee's relationship, if one is asked for, and returns its audit event.
pub(crate) async fn create_for_invitee(
    txn: &DatabaseTransaction,
    actor: Actor,
    organization_id: Id,
    invitee_id: Id,
    relationship: Option<SeatedInvitee>,
) -> Result<Option<NewEvent>, Error> {
    let Some(relationship) = relationship else {
        return Ok(None);
    };
    let settled = create(txn, organization_id, relationship.draft_for(invitee_id)).await?;
    Ok(creation_event(actor, &settled))
}

/// The audit event for `actor`'s create: `relationship_created` when it inserted the
/// relationship, none when it returned the pair's existing one.
fn creation_event(actor: Actor, settled: &Settled) -> Option<NewEvent> {
    settled
        .is_new
        .then(|| audit_event::relationship_created(actor, &settled.relationship.relationship()))
}

/// The relationship with its participants' names, as `caller` sees it; `None` if there is none.
pub async fn get_relationship_with_user_names_for_caller(
    db: &DatabaseConnection,
    caller: &users::Model,
    relationship_id: Id,
) -> Result<Option<CoachingRelationshipWithUserNames>, Error> {
    Ok(get_relationship_with_user_names(db, relationship_id)
        .await?
        .map(|relationship| relationship.for_caller(caller)))
}

/// The relationships `caller` is in, narrowed to those in which they hold `role` when one is
/// given, with their participants' names, each as `caller` sees it.
pub async fn find_by_user_id_with_user_names_for_caller(
    db: &DatabaseConnection,
    caller: &users::Model,
    role: Option<Role>,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    Ok(
        find_by_user_id_with_user_names(db, &visibility_for(caller), role)
            .await?
            .into_iter()
            .map(|relationship| relationship.for_caller(caller))
            .collect(),
    )
}

/// Finds coaching relationships for `caller` within an organization, respecting role-based
/// access, each as `caller` sees it.
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
    caller: &users::Model,
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
    let is_admin = entity_api::user::has_admin_access(&txn, caller.id, organization_id)
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
        find_by_user_and_organization_with_user_names(&txn, caller.id, organization_id).await?
    };

    // Commit transaction
    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    Ok(coaching_relationships
        .into_iter()
        .map(|relationship| relationship.for_caller(caller))
        .collect())
}

/// Whichever of `coach` and `coachee` owns the schedule of a session scheduled by
/// `scheduler_id`, under the rules of `kind`.
pub(crate) fn schedule_owner<'a>(
    kind: Kind,
    coach: &'a users::Model,
    coachee: &'a users::Model,
    scheduler_id: Id,
) -> &'a users::Model {
    let scheduler_seat = [(coach, Seat::CoachId), (coachee, Seat::CoacheeId)]
        .into_iter()
        .find_map(|(user, seat)| (user.id == scheduler_id).then_some(seat));
    match kind.rules().schedule_owner().seat(scheduler_seat) {
        Seat::CoachId => coach,
        Seat::CoacheeId => coachee,
    }
}

/// A 409 for the first participant, coach seat first, whose seat must still be in the
/// organization for sessions to be scheduled or changed but who is not among `member_ids`,
/// the relationship's participants still in its organization.
pub(crate) fn ensure_required_participants_are_members(
    relationship: &Model,
    member_ids: &[Id],
) -> Result<(), Error> {
    relationship
        .seat_blocking_scheduling(member_ids)
        .map_or(Ok(()), |seat| {
            Err(participant_not_a_member(relationship, seat))
        })
}

/// The 409 for a relationship whose participant in `seat` no longer belongs to its
/// organization, its message naming that participant's role.
pub(crate) fn participant_not_a_member(relationship: &Model, seat: Seat) -> Error {
    let message = format!(
        "Your {} is no longer a member of this organization. Re-add them to the organization first.",
        relationship.kind.rules().role(seat).as_str()
    );
    not_a_member(
        &message,
        reason::PARTICIPANT_NOT_A_MEMBER,
        relationship.organization_id,
        relationship.participant(seat),
    )
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
    ensure_members(
        db,
        relationship,
        assignee_ids,
        "An assignee is no longer a member of this organization; re-add them to the \
         organization first.",
        reason::ASSIGNEE_NOT_A_MEMBER,
    )
    .await
}

/// A 409 carrying `reason` and `message` for the first of `user_ids`, in order, no longer in
/// the relationship's organization.
async fn ensure_members(
    db: &impl ConnectionTrait,
    relationship: &Model,
    user_ids: &[Id],
    message: &str,
    reason: &str,
) -> Result<(), Error> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let members = retain_organization_members(db, user_ids, relationship.organization_id).await?;
    user_ids
        .iter()
        .find(|id| !members.contains(id))
        .map_or(Ok(()), |&user_id| {
            Err(not_a_member(
                message,
                reason,
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
    use sea_orm::Iterable;

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
        assert_eq!(details["reason"], "participant_not_a_member");
        assert_eq!(
            details["user_id"],
            serde_json::json!(relationship.coachee_id)
        );
    }

    /// KIND-24: for every kind and every set of participants still in the organization,
    /// `scheduling_blocked_by` names the user the scheduling 409 names, and `None` when it passes.
    #[test]
    fn scheduling_blocked_by_names_whom_the_scheduling_conflict_names() {
        for kind in Kind::iter() {
            let relationship = Model {
                kind,
                ..relationship()
            };
            let caller = crate::test_utils::proof::member_coach(&relationship);
            for (coach_is_member, coachee_is_member) in
                [(true, true), (true, false), (false, true), (false, false)]
            {
                let membership = Membership {
                    coach_is_member,
                    coachee_is_member,
                };
                let refused = ensure_required_participants_are_members(
                    &relationship,
                    &membership.member_ids(&relationship),
                )
                .err()
                .map(|err| match err.error_kind {
                    DomainErrorKind::Internal(InternalErrorKind::Entity(
                        EntityErrorKind::Conflict {
                            details: Some(details),
                            ..
                        },
                    )) => details["user_id"].clone(),
                    other => panic!("expected a conflict, got {other:?}"),
                });

                assert_eq!(
                    Capabilities::for_caller(&relationship, membership, &caller)
                        .scheduling_blocked_by
                        .map(|user_id| serde_json::json!(user_id)),
                    refused,
                    "{kind:?} {membership:?}"
                );
            }
        }
    }

    /// For every kind and every requested role, the invitee takes a seat holding that role
    /// (the coachee seat when none is requested) or the request is refused as
    /// `role_not_in_kind`; the other person always takes the other seat.
    #[test]
    fn an_invitee_takes_the_seat_holding_their_requested_role() {
        let (invitee, other) = (Id::new_v4(), Id::new_v4());
        for kind in Kind::iter() {
            let rules = kind.rules();
            for role in Role::iter().map(Some).chain([None]) {
                let seated = InviteeRelationship {
                    with_user_id: other,
                    kind,
                    role,
                }
                .seated()
                .map(|seated| seated.draft_for(invitee));
                let role_in_kind = role.is_none_or(|role| {
                    [Seat::CoachId, Seat::CoacheeId]
                        .into_iter()
                        .any(|seat| rules.role(seat) == role)
                });

                match seated {
                    Ok(draft) => {
                        assert!(role_in_kind, "{kind:?} {role:?}: refused");
                        assert_eq!(draft.kind, kind);
                        let (invitee_seat, other_id) = if draft.coach_id == invitee {
                            (Seat::CoachId, draft.coachee_id)
                        } else {
                            assert_eq!(draft.coachee_id, invitee, "{kind:?} {role:?}");
                            (Seat::CoacheeId, draft.coach_id)
                        };
                        assert_eq!(other_id, other, "{kind:?} {role:?}");
                        assert_eq!(
                            rules.role(invitee_seat),
                            role.unwrap_or(rules.role(Seat::CoacheeId)),
                            "{kind:?} {role:?}"
                        );
                        assert!(
                            invitee_seat == Seat::CoacheeId
                                || rules.role(Seat::CoacheeId) != rules.role(invitee_seat),
                            "{kind:?} {role:?}: the coachee seat when it holds the role"
                        );
                    }
                    Err(err) => {
                        assert!(!role_in_kind, "{kind:?} {role:?}: accepted");
                        let DomainErrorKind::Internal(InternalErrorKind::Entity(
                            EntityErrorKind::Unprocessable { details, .. },
                        )) = err.error_kind
                        else {
                            panic!("expected a 422, got {:?}", err.error_kind);
                        };
                        assert_eq!(details["reason"], "role_not_in_kind");
                    }
                }
            }
        }
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
