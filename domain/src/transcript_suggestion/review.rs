//! Reviewing action suggestions: list, accept, dismiss and restore, and scanning for them.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use log::warn;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{DatabaseConnection, TransactionTrait};
use serde::{Deserialize, Serialize};
use service::config::Config;
use utoipa::ToSchema;

use super::due_date::names_next_session;
use super::extract::notify;
use super::{
    find_by_session, transition, validation, ActionProposal, Actions, Model, People, Status,
    Suggestible, Target,
};
use crate::action::{self, ActionWithAssignees};
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::events::EventPublisher;
use crate::status::Status as ActionStatus;
use crate::transcript_analysis::Status as AnalysisStatus;
use crate::transcript_export::{LabeledSegment, SpeakerRole};
use crate::{
    actions, coaching_session, coaching_sessions, emails, goal, transcript_analysis, transcription,
    users, Id,
};

/// Days after the session, or after now once it has started, until a suggestion without a due
/// date is due when no later session is scheduled.
const FALLBACK_DUE_DAYS: i64 = 7;

/// Who is acting in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    Coach,
    Coachee,
}

/// Where a default due date came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = domain::transcript_suggestion::review::DueSource)]
pub enum DueSource {
    NextSession,
    /// A week after a session that has not started yet.
    WeekAfterSession,
    /// A week from now.
    SevenDays,
}

/// Why a spoken deadline gave no due date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = domain::transcript_suggestion::review::UndatedReason)]
pub enum UndatedReason {
    /// It names the next session, and none is scheduled.
    NoNextSession,
    /// It is vague or ambiguous, such as "soon" or "next week".
    NotADate,
}

/// The due date accept applies when a suggestion has none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = domain::transcript_suggestion::review::DefaultDue)]
pub struct DefaultDue {
    #[schema(value_type = String, format = DateTime)]
    pub at: DateTimeWithTimeZone,
    pub source: DueSource,
}

/// What a reviewer may change on an action suggestion.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = domain::transcript_suggestion::review::ActionEdits)]
pub struct ActionEdits {
    pub body: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub due_by: Option<DateTimeWithTimeZone>,
    pub assignee_ids: Vec<Id>,
    /// The goal to link the action to; `null` or absent for none.
    #[serde(default)]
    pub goal_id: Option<Id>,
}

impl ActionEdits {
    /// `stored` with its body, due date, goal and assignees replaced by these edits.
    pub fn onto(&self, stored: &ActionProposal) -> ActionProposal {
        ActionProposal {
            action: ActionWithAssignees {
                action: actions::Model {
                    body: Some(self.body.clone()),
                    due_by: self.due_by,
                    goal_id: self.goal_id,
                    ..stored.action.action.clone()
                },
                assignee_ids: self.assignee_ids.clone(),
            },
            due_phrase: stored.due_phrase.clone(),
        }
    }
}

/// An action suggestion as reviewers see it.
#[derive(Clone, Debug, PartialEq, Serialize, ToSchema)]
#[schema(as = domain::transcript_suggestion::review::ActionSuggestion)]
pub struct ActionSuggestion {
    pub id: Id,
    pub coaching_session_id: Id,
    /// The action accept will create, with `due_phrase` as the spoken deadline.
    pub proposal: ActionProposal,
    pub quote: String,
    pub segment: SourceSegment,
    /// The existing action this may repeat; `null` when not flagged or that action is gone.
    pub duplicate_of: Option<DuplicateOf>,
    pub default_due: DefaultDue,
    /// Why the spoken `due_phrase` gave no due date; `null` when it did, or when none was spoken.
    pub undated_reason: Option<UndatedReason>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,
    /// Who dismissed or accepted the suggestion; `null` while pending.
    pub resolved_by: Option<Id>,
    /// When it was dismissed or accepted; `null` while pending.
    #[schema(value_type = Option<String>, format = DateTime)]
    pub resolved_at: Option<DateTimeWithTimeZone>,
}

/// The transcript segment a suggestion came from.
#[derive(Clone, Debug, PartialEq, Serialize, ToSchema)]
#[schema(as = domain::transcript_suggestion::review::SourceSegment)]
pub struct SourceSegment {
    pub id: Id,
    pub start_ms: i32,
    pub speaker_role: Option<SpeakerRole>,
    pub speaker_label: String,
}

/// An existing action a suggestion may repeat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = domain::transcript_suggestion::review::DuplicateOf)]
pub struct DuplicateOf {
    pub action_id: Id,
    pub body: Option<String>,
}

/// Where scanning the session's latest transcript for suggested actions stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = domain::transcript_suggestion::review::ScanStatus)]
pub enum ScanStatus {
    /// Suggestions are not configured, or the session has no completed transcript.
    Unavailable,
    /// The latest completed transcript was never scanned.
    None,
    Pending,
    Processing,
    Completed,
    Failed,
}

impl From<AnalysisStatus> for ScanStatus {
    fn from(status: AnalysisStatus) -> Self {
        match status {
            AnalysisStatus::Pending => Self::Pending,
            AnalysisStatus::Processing => Self::Processing,
            AnalysisStatus::Completed => Self::Completed,
            AnalysisStatus::Failed => Self::Failed,
        }
    }
}

/// A session's scan state as clients see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = domain::transcript_suggestion::review::Scan)]
pub struct Scan {
    pub status: ScanStatus,
}

/// Which participant `user_id` is; `None` when neither.
pub fn actor_of(user_id: Id, people: People) -> Option<Actor> {
    if user_id == people.coach_id {
        Some(Actor::Coach)
    } else if user_id == people.coachee_id {
        Some(Actor::Coachee)
    } else {
        None
    }
}

/// Whether `actor` may act on a suggestion assigned to `stored`, optionally changing it to `requested`.
///
/// The coach may always act. The coachee may not touch a coach-only suggestion, nor add or
/// remove the coach.
pub fn may_act(actor: Actor, people: People, stored: &[Id], requested: Option<&[Id]>) -> bool {
    let has_coach = |ids: &[Id]| ids.contains(&people.coach_id);
    match actor {
        Actor::Coach => true,
        Actor::Coachee => {
            let coach_only = has_coach(stored) && !stored.contains(&people.coachee_id);
            let moves_coach = requested.is_some_and(|r| has_coach(r) != has_coach(stored));
            !coach_only && !moves_coach
        }
    }
}

/// The due date accept applies when a suggestion has none.
///
/// The next session's start when it has not started yet, otherwise seven days after the later of
/// this session's start and `now`.
pub async fn default_due(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    now: DateTime<Utc>,
) -> Result<DefaultDue, Error> {
    let next =
        coaching_session::find_next_session(db, session.coaching_relationship_id, session.date)
            .await?;
    Ok(next
        .map(|n| n.date.and_utc())
        .filter(|start| *start > now)
        .map(|start| DefaultDue {
            at: start.fixed_offset(),
            source: DueSource::NextSession,
        })
        .unwrap_or_else(|| {
            let start = session.date.and_utc();
            let (from, source) = if start > now {
                (start, DueSource::WeekAfterSession)
            } else {
                (now, DueSource::SevenDays)
            };
            DefaultDue {
                at: (from + Duration::days(FALLBACK_DUE_DAYS)).fixed_offset(),
                source,
            }
        }))
}

/// The session's action suggestions in `status`, oldest first.
///
/// A row whose proposal or segment cannot be read is skipped with a warning.
pub async fn list_actions(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    status: Status,
    now: DateTime<Utc>,
) -> Result<Vec<ActionSuggestion>, Error> {
    let rows = find_by_session(db, session.id, Target::Action, status).await?;
    if rows.is_empty() {
        return Ok(vec![]);
    }

    let default_due = default_due(db, session, now).await?;
    let segments = segments_of(db, session, &rows).await?;
    let duplicates = duplicates_of(db, session.id, &rows).await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let proposal = read_proposal(&row)
                .inspect_err(|_| warn!("Suggestion {} has an unreadable proposal", row.id))
                .ok()?;
            let Some(segment) = segments.get(&row.segment_id) else {
                warn!("Suggestion {} has no readable segment", row.id);
                return None;
            };
            Some(ActionSuggestion {
                undated_reason: undated_reason(&proposal, &default_due),
                id: row.id,
                coaching_session_id: row.coaching_session_id,
                proposal,
                quote: row.quote,
                segment: SourceSegment {
                    id: segment.id,
                    start_ms: segment.start_ms,
                    speaker_role: segment.speaker_role,
                    speaker_label: segment.speaker_label.clone(),
                },
                duplicate_of: row
                    .duplicate_of_id
                    .and_then(|id| duplicates.get(&id).cloned()),
                default_due: default_due.clone(),
                created_at: row.created_at,
                resolved_by: row.resolved_by,
                resolved_at: row.resolved_at,
            })
        })
        .collect())
}

/// Why a spoken deadline gave no due date; `None` when it did, or nothing was spoken.
///
/// A phrase naming the next session resolves once one is scheduled, so it then has no reason.
fn undated_reason(proposal: &ActionProposal, default_due: &DefaultDue) -> Option<UndatedReason> {
    let phrase = proposal
        .due_phrase
        .as_deref()
        .filter(|_| proposal.action.action.due_by.is_none())?;
    match (names_next_session(phrase), default_due.source) {
        (true, DueSource::NextSession) => None,
        (true, _) => Some(UndatedReason::NoNextSession),
        (false, _) => Some(UndatedReason::NotADate),
    }
}

/// Creates the suggested action, applying `edits` when given, and marks the suggestion accepted.
///
/// The accepting user owns the action. Without a due date it gets `default_due`. The action and
/// the transition commit together, so a suggestion accepted twice creates one action.
pub async fn accept_action(
    db: &DatabaseConnection,
    publisher: &EventPublisher,
    config: &Config,
    user: &users::Model,
    suggestion: Model,
    edits: Option<ActionEdits>,
    now: DateTime<Utc>,
) -> Result<ActionWithAssignees, Error> {
    let stored = read_proposal(&suggestion)?;
    let requested = edits.as_ref().map(|e| e.assignee_ids.as_slice());
    let (session, people) = authorize(db, user, &suggestion, &stored, requested).await?;
    let proposed = match edits {
        Some(edits) => revise(edits.onto(&stored), &stored, people)?,
        None => stored.action,
    };
    check_goal(db, &session, proposed.action.goal_id).await?;

    let due_by = match proposed.action.due_by {
        Some(due_by) => due_by,
        None => default_due(db, &session, now).await?.at,
    };
    let model = actions::Model {
        coaching_session_id: suggestion.coaching_session_id,
        due_by: Some(due_by),
        status: ActionStatus::NotStarted,
        ..proposed.action
    };

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    let created = entity_api::action::create_with_assignees(
        &txn,
        model,
        user.id,
        Some(proposed.assignee_ids),
    )
    .await?;
    let accepted = transition(
        &txn,
        suggestion.id,
        Status::Pending,
        Status::Accepted,
        user.id,
        Some(created.action.id),
        now.fixed_offset(),
    )
    .await?;
    if accepted.is_none() {
        txn.rollback()
            .await
            .map_err(entity_api::error::Error::from)?;
        return Err(conflict("Only a pending suggestion can be accepted."));
    }
    txn.commit().await.map_err(entity_api::error::Error::from)?;

    action::publish_action_changed(db, publisher, &created, true).await;
    if created.has_assignees() {
        emails::notify_action_assigned(db, config, &created.assignee_ids, user, &created.action)
            .await;
    }
    notify::<Actions>(db, publisher, suggestion.coaching_session_id).await;
    Ok(created)
}

/// Dismisses a pending suggestion.
pub async fn dismiss(
    db: &DatabaseConnection,
    publisher: &EventPublisher,
    user: &users::Model,
    suggestion: Model,
    now: DateTime<Utc>,
) -> Result<Model, Error> {
    resolve(
        db,
        publisher,
        user,
        suggestion,
        (Status::Pending, Status::Dismissed),
        now,
        "Only a pending suggestion can be dismissed.",
    )
    .await
}

/// Returns a dismissed suggestion to pending.
pub async fn restore(
    db: &DatabaseConnection,
    publisher: &EventPublisher,
    user: &users::Model,
    suggestion: Model,
    now: DateTime<Utc>,
) -> Result<Model, Error> {
    resolve(
        db,
        publisher,
        user,
        suggestion,
        (Status::Dismissed, Status::Pending),
        now,
        "Only a dismissed suggestion can be restored.",
    )
    .await
}

/// Moves a suggestion between statuses after the permission check.
async fn resolve(
    db: &DatabaseConnection,
    publisher: &EventPublisher,
    user: &users::Model,
    suggestion: Model,
    (from, to): (Status, Status),
    now: DateTime<Utc>,
    refusal: &str,
) -> Result<Model, Error> {
    let stored = read_proposal(&suggestion)?;
    authorize(db, user, &suggestion, &stored, None).await?;

    let moved = transition(
        db,
        suggestion.id,
        from,
        to,
        user.id,
        None,
        now.fixed_offset(),
    )
    .await?
    .ok_or_else(|| conflict(refusal))?;
    notify::<Actions>(db, publisher, suggestion.coaching_session_id).await;
    Ok(moved)
}

/// Where scanning the session's latest completed transcript for suggested actions stands.
pub async fn scan_status(
    db: &DatabaseConnection,
    config: &Config,
    session: &coaching_sessions::Model,
) -> Result<ScanStatus, Error> {
    Ok(match scannable(db, config, session).await? {
        None => ScanStatus::Unavailable,
        Some((_, None)) => ScanStatus::None,
        Some((_, Some(analysis))) => analysis.status.into(),
    })
}

/// Queues a scan of the session's latest completed transcript that never ran or failed.
///
/// A scan that is running or already completed is never repeated.
pub async fn start_scan(
    db: &DatabaseConnection,
    publisher: &EventPublisher,
    config: &Config,
    session: &coaching_sessions::Model,
) -> Result<ScanStatus, Error> {
    let (transcription_id, analysis) = scannable(db, config, session)
        .await?
        .ok_or_else(|| validation("Transcript suggestions are not available for this session."))?;

    match analysis.map(|a| (a.id, a.status)) {
        None => queue_scan(db, transcription_id).await?,
        Some((id, AnalysisStatus::Failed)) => {
            transcript_analysis::restart_failed(db, id, Utc::now().fixed_offset())
                .await?
                .ok_or_else(|| conflict(SCAN_RUNNING))?;
        }
        Some((_, AnalysisStatus::Pending | AnalysisStatus::Processing)) => {
            return Err(conflict(SCAN_RUNNING))
        }
        Some((_, AnalysisStatus::Completed)) => {
            return Err(conflict("Transcript already scanned for actions."))
        }
    }

    notify::<Actions>(db, publisher, session.id).await;
    Ok(ScanStatus::Pending)
}

const SCAN_RUNNING: &str = "A scan is already running.";

/// The latest transcription's id and its action analysis, or `None` when nothing can be scanned.
async fn scannable(
    db: &DatabaseConnection,
    config: &Config,
    session: &coaching_sessions::Model,
) -> Result<Option<(Id, Option<transcript_analysis::Model>)>, Error> {
    if config.assembly_ai_api_key().is_none() {
        return Ok(None);
    }
    let Some(latest) = transcription::find_by_coaching_session(db, session.id)
        .await?
        .filter(|t| t.status == transcription::TranscriptionStatus::Completed)
    else {
        return Ok(None);
    };
    let analysis =
        transcript_analysis::find_by_transcription_and_target(db, latest.id, Target::Action)
            .await?;
    Ok(Some((latest.id, analysis)))
}

/// Records a pending action analysis; one created meanwhile by another request is a conflict.
async fn queue_scan(db: &DatabaseConnection, transcription_id: Id) -> Result<(), Error> {
    let Err(e) = transcript_analysis::create_pending(db, transcription_id, Target::Action).await
    else {
        return Ok(());
    };
    match transcript_analysis::find_by_transcription_and_target(
        db,
        transcription_id,
        Target::Action,
    )
    .await?
    {
        Some(_) => Err(conflict(SCAN_RUNNING)),
        None => Err(e.into()),
    }
}

/// Loads the suggestion's session and people, and rejects a user who may not act on it.
async fn authorize(
    db: &DatabaseConnection,
    user: &users::Model,
    suggestion: &Model,
    stored: &ActionProposal,
    requested: Option<&[Id]>,
) -> Result<(coaching_sessions::Model, People), Error> {
    let (session, relationship) =
        coaching_session::find_by_id_with_coaching_relationship(db, suggestion.coaching_session_id)
            .await?;
    let people = People {
        coach_id: relationship.coach_id,
        coachee_id: relationship.coachee_id,
    };
    actor_of(user.id, people)
        .filter(|actor| may_act(*actor, people, Actions::assignees(stored), requested))
        .map(|_| (session, people))
        .ok_or_else(forbidden)
}

/// Rejects a goal outside the session's coaching relationship; a missing goal is rejected alike.
async fn check_goal(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    goal_id: Option<Id>,
) -> Result<(), Error> {
    let Some(goal_id) = goal_id else {
        return Ok(());
    };
    let in_relationship = match goal::find_by_id(db, goal_id).await.map_err(Error::from) {
        Ok(found) => found.coaching_relationship_id == session.coaching_relationship_id,
        Err(e) if is_not_found(&e) => false,
        Err(e) => return Err(e),
    };
    in_relationship
        .then_some(())
        .ok_or_else(|| validation("The goal is not in this coaching relationship."))
}

/// The action `edits` describe on top of `stored`, with a trimmed body and no repeated assignee.
fn revise(
    edits: ActionProposal,
    stored: &ActionProposal,
    people: People,
) -> Result<ActionWithAssignees, Error> {
    Actions::check(&edits, people)?;
    let assignee_ids = edits
        .action
        .assignee_ids
        .iter()
        .fold(Vec::new(), |mut ids, id| {
            if !ids.contains(id) {
                ids.push(*id);
            }
            ids
        });

    Ok(ActionWithAssignees {
        action: actions::Model {
            body: edits.action.action.body.map(|b| b.trim().to_string()),
            due_by: edits.action.action.due_by,
            goal_id: edits.action.action.goal_id,
            status: ActionStatus::NotStarted,
            ..stored.action.action.clone()
        },
        assignee_ids,
    })
}

/// Labeled segments by id, read once per transcription the rows came from.
async fn segments_of(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    rows: &[Model],
) -> Result<HashMap<Id, LabeledSegment>, Error> {
    let analysis_ids: HashSet<Id> = rows.iter().map(|r| r.transcript_analysis_id).collect();
    let mut transcription_ids = HashSet::new();
    for analysis_id in analysis_ids {
        transcription_ids.insert(
            transcript_analysis::find_by_id(db, analysis_id)
                .await?
                .transcription_id,
        );
    }

    let mut segments = HashMap::new();
    for transcription_id in transcription_ids {
        segments.extend(
            transcription::read_segments(db, session, transcription_id)
                .await?
                .into_iter()
                .map(|s| (s.id, s)),
        );
    }
    Ok(segments)
}

/// The session's actions that some row flags as a duplicate, by id.
async fn duplicates_of(
    db: &DatabaseConnection,
    coaching_session_id: Id,
    rows: &[Model],
) -> Result<HashMap<Id, DuplicateOf>, Error> {
    let flagged: HashSet<Id> = rows.iter().filter_map(|r| r.duplicate_of_id).collect();
    if flagged.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(action::find_by_coaching_session(db, coaching_session_id)
        .await?
        .into_iter()
        .filter(|a| flagged.contains(&a.id))
        .map(|a| {
            (
                a.id,
                DuplicateOf {
                    action_id: a.id,
                    body: a.body,
                },
            )
        })
        .collect())
}

fn read_proposal(suggestion: &Model) -> Result<ActionProposal, Error> {
    serde_json::from_value(suggestion.proposal.clone()).map_err(|e| {
        internal(format!(
            "Suggestion {} has an unreadable proposal: {e}",
            suggestion.id
        ))
    })
}

fn is_not_found(error: &Error) -> bool {
    matches!(
        error.error_kind,
        DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound))
    )
}

fn forbidden() -> Error {
    entity_error(EntityErrorKind::Forbidden)
}

fn conflict(message: &str) -> Error {
    entity_error(EntityErrorKind::Conflict {
        message: message.to_string(),
        details: None,
    })
}

fn entity_error(kind: EntityErrorKind) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(kind)),
    }
}

fn internal(message: String) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(message)),
    }
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "review_sqlite_tests.rs"]
mod sqlite_tests;
