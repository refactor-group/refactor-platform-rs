//! An administrator changing what a relationship is, in place, once they have confirmed what
//! the change does to each participant.

use std::collections::BTreeMap;

use chrono::{TimeDelta, Utc};
use entity_api::{coaching_relationship, coaching_session};
use sea_orm::{DatabaseConnection, TransactionTrait};
use serde::Serialize;
use service::config::Config;
use utoipa::ToSchema;

use super::{
    ensure_members, get_relationship_with_user_names_for_caller, CoachingRelationshipWithUserNames,
    Draft,
};
use crate::action::{find_by_coaching_relationship, CallerVisibility, FindByRelationshipParams};
use crate::coaching_relationship_kind::Kind;
use crate::coaching_relationships::Model;
use crate::error::{reason, DomainErrorKind, Error, InternalErrorKind};
use crate::{users, Id};

/// What an administrator asks a relationship to become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub kind: Kind,
    /// Who becomes the coach, for a kind whose seats differ; `None` otherwise.
    pub coach_id: Option<Id>,
    /// Whether the administrator has seen and accepted the switch's [`Consequences`].
    pub confirmed: bool,
}

/// What a switch changes for the relationship's participants, sent back in a 409 until the
/// administrator confirms it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = domain::coaching_relationship::kind_switch::Consequences)]
pub struct Consequences {
    /// Each participant who will see actions they cannot see now, with how many of them per
    /// assignee. Empty when nobody gains sight of any.
    pub actions_becoming_visible: Vec<ActionsBecomingVisible>,
    /// The participants who gain the right to schedule, reschedule and cancel sessions, run
    /// series, set meeting links and delete any topic.
    pub scheduling_gained: Vec<Id>,
    /// The participants who lose that right. What they already created stays.
    pub scheduling_lost: Vec<Id>,
}

/// The actions one participant will see after the switch and cannot see now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = domain::coaching_relationship::kind_switch::ActionsBecomingVisible)]
pub struct ActionsBecomingVisible {
    /// The participant who gains sight of them.
    pub user_id: Id,
    /// How many of them each assignee holds. An action with two assignees counts once for each.
    pub by_assignee: Vec<AssigneeActionCount>,
}

/// How many of the actions becoming visible one assignee holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = domain::coaching_relationship::kind_switch::AssigneeActionCount)]
pub struct AssigneeActionCount {
    /// The assignee.
    pub assignee_id: Id,
    /// How many of the actions becoming visible are assigned to them.
    pub count: u64,
}

/// Switches relationship `relationship_id` of `organization_id` to the requested kind, and
/// returns it as `caller` sees it.
///
/// Refuses, changing nothing: a relationship not in the organization (404); a `coach_id`
/// missing, out of place, or not a participant (422); a participant no longer in the
/// organization (409 `participant_not_a_member`); a pair whose other relationship leaves no
/// room for the switch (409 `pair_already_related`); and, until `confirmed`, every switch
/// (409 `kind_switch_needs_confirmation`, its details the [`Consequences`]). Asking for what
/// the relationship already is changes nothing and succeeds.
///
/// A confirmed switch updates the relationship and, in the same transaction, marks as already
/// sent the reminders of anyone it newly reminds for sessions already inside the reminder
/// window, so it sends no surprise reminder.
pub async fn switch_for_caller(
    db: &DatabaseConnection,
    config: &Config,
    caller: &users::Model,
    organization_id: Id,
    relationship_id: Id,
    request: Request,
) -> Result<CoachingRelationshipWithUserNames, Error> {
    let relationship =
        coaching_relationship::find_by_id_in_organization(db, organization_id, relationship_id)
            .await?;
    let target = target_of(&relationship, request)?;

    if target != draft_of(&relationship) {
        let switched = Model {
            coach_id: target.coach_id,
            coachee_id: target.coachee_id,
            kind: target.kind,
            ..relationship.clone()
        };
        ensure_members(
            db,
            &relationship,
            &participants(&relationship),
            "A participant is no longer a member of this organization. Re-add them to the \
             organization first.",
            reason::PARTICIPANT_NOT_A_MEMBER,
        )
        .await?;
        coaching_relationship::ensure_pair_allows(db, &relationship, target).await?;
        if !request.confirmed {
            return Err(needs_confirmation(
                consequences(db, &relationship, &switched).await?,
            ));
        }
        apply(db, config, &relationship, &switched, target).await?;
    }

    get_relationship_with_user_names_for_caller(db, caller, relationship.id)
        .await?
        .ok_or_else(Error::not_found)
}

/// The seats and kind `request` asks of `relationship`: the chosen coach and the other
/// participant for a kind whose seats differ, today's seats for one whose seats are equal.
fn target_of(relationship: &Model, request: Request) -> Result<Draft, Error> {
    let draft = |coach_id: Id, coachee_id: Id| Draft {
        coach_id,
        coachee_id,
        kind: request.kind,
    };
    match (request.kind.seats_differ(), request.coach_id) {
        (true, Some(coach_id)) => relationship
            .seat(coach_id)
            .map(|_| {
                let other = [relationship.coach_id, relationship.coachee_id]
                    .into_iter()
                    .find(|&id| id != coach_id)
                    .unwrap_or(coach_id);
                draft(coach_id, other)
            })
            .ok_or_else(|| {
                Error::invalid_because(
                    reason::COACH_NOT_A_PARTICIPANT,
                    "The coach must be one of the relationship's two participants.",
                )
            }),
        (true, None) => Err(Error::invalid_because(
            reason::COACH_REQUIRED,
            "Name which participant becomes the coach.",
        )),
        (false, Some(_)) => Err(Error::invalid_because(
            reason::COACH_NOT_APPLICABLE,
            "This kind has no coach; leave out coach_id.",
        )),
        (false, None) => Ok(draft(relationship.coach_id, relationship.coachee_id)),
    }
}

/// `relationship`'s seats and kind as they stand.
fn draft_of(relationship: &Model) -> Draft {
    Draft {
        coach_id: relationship.coach_id,
        coachee_id: relationship.coachee_id,
        kind: relationship.kind,
    }
}

/// The relationship's participants, coach seat first, each once.
fn participants(relationship: &Model) -> Vec<Id> {
    std::iter::once(relationship.coach_id)
        .chain(
            (relationship.coachee_id != relationship.coach_id).then_some(relationship.coachee_id),
        )
        .collect()
}

/// What switching `relationship` to `switched` changes for its participants, read from the
/// rules of each kind.
async fn consequences(
    db: &DatabaseConnection,
    relationship: &Model,
    switched: &Model,
) -> Result<Consequences, Error> {
    let actions =
        find_by_coaching_relationship(db, relationship.id, FindByRelationshipParams::default())
            .await?;
    let participants = participants(relationship);
    let actions_becoming_visible = participants
        .iter()
        .filter_map(|&user_id| {
            let sees = |model: &Model, assignee_ids: &[Id]| {
                CallerVisibility::for_relationship(user_id, model)
                    .is_some_and(|visibility| visibility.allows(assignee_ids))
            };
            let by_assignee: BTreeMap<Id, u64> = actions
                .iter()
                .filter(|action| {
                    sees(switched, &action.assignee_ids)
                        && !sees(relationship, &action.assignee_ids)
                })
                .flat_map(|action| action.assignee_ids.iter().copied())
                .fold(BTreeMap::new(), |mut counts, assignee_id| {
                    *counts.entry(assignee_id).or_default() += 1;
                    counts
                });
            (!by_assignee.is_empty()).then(|| ActionsBecomingVisible {
                user_id,
                by_assignee: by_assignee
                    .into_iter()
                    .map(|(assignee_id, count)| AssigneeActionCount { assignee_id, count })
                    .collect(),
            })
        })
        .collect();
    let scheduling = |before: bool| {
        participants
            .iter()
            .copied()
            .filter(|&id| {
                relationship.can_schedule_sessions(id) == before
                    && switched.can_schedule_sessions(id) != before
            })
            .collect()
    };

    Ok(Consequences {
        actions_becoming_visible,
        scheduling_gained: scheduling(false),
        scheduling_lost: scheduling(true),
    })
}

/// The 409 asking the administrator to confirm `consequences` before the switch applies.
fn needs_confirmation(consequences: Consequences) -> Error {
    match serde_json::to_value(consequences) {
        Ok(details) => Error::conflict_because(
            reason::KIND_SWITCH_NEEDS_CONFIRMATION,
            "Switching this relationship's kind changes what its participants can see and do. \
             Confirm to switch.",
            details,
        ),
        Err(err) => Error {
            source: Some(Box::new(err)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "the switch's consequences did not serialize".to_string(),
            )),
        },
    }
}

/// Switches `relationship` to `target` and marks as sent the reminders `switched` newly owes
/// for sessions inside the reminder window, in one transaction.
async fn apply(
    db: &DatabaseConnection,
    config: &Config,
    relationship: &Model,
    switched: &Model,
    target: Draft,
) -> Result<(), Error> {
    let newly_reminded: Vec<Id> = participants(relationship)
        .into_iter()
        .filter(|&id| !relationship.is_reminded(id) && switched.is_reminded(id))
        .collect();
    // An out-of-range lead turns the sweep off, so there is nothing to suppress.
    let lead = TimeDelta::from_std(config.session_reminder_lead()).unwrap_or_default();
    let now = Utc::now().naive_utc();

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    coaching_relationship::change_kind(&txn, relationship, target).await?;
    coaching_session::mark_reminders_sent(
        &txn,
        relationship.id,
        &newly_reminded,
        (now, now + lead),
    )
    .await?;
    txn.commit().await.map_err(entity_api::error::Error::from)?;
    Ok(())
}
