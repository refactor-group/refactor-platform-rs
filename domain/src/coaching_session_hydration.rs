use crate::coaching_relationships;
use crate::coaching_sessions::Model;
use crate::error::Error;
use crate::events::DomainEvent;
use crate::topic_status::Status;
use crate::{coaching_session_topics, goals, Id};
use chrono::NaiveDateTime;
use entity_api::coaching_session;
use entity_api::coaching_session_goal;
use entity_api::coaching_session_topic;
use entity_api::goal;
use log::*;
use sea_orm::{ConnectionTrait, DatabaseTransaction};

/// Context handed to each hydration task. Tasks do their DB writes on `txn` (so
/// they commit atomically with the session row) and return the `DomainEvent`s to
/// publish AFTER commit. Fields are the minimum the current task set needs; this
/// struct grows by a field when a future task needs more (e.g. `db`/`config`).
pub(crate) struct CoachingSessionHydrationContext<'a> {
    pub txn: &'a DatabaseTransaction,
    pub session: &'a Model,
    pub relationship: &'a coaching_relationships::Model,
    /// The relationship's participants still in its organization, checked once by the caller.
    pub member_ids: &'a [Id],
}

/// A unit of deferred, prerequisite work run at-latest on a coaching session's
/// first read (or eagerly at create), inside the hydration transaction. Loosely
/// coupled: registering one requires no change to `create`/`ensure_hydrated`.
#[async_trait::async_trait]
pub(crate) trait CoachingSessionHydrationTask: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(
        &self,
        ctx: &CoachingSessionHydrationContext<'_>,
    ) -> Result<Vec<DomainEvent>, Error>;
}

/// Ordered registry of hydration tasks.
fn coaching_session_hydration_tasks() -> Vec<Box<dyn CoachingSessionHydrationTask>> {
    vec![
        Box::new(GoalsCarryForwardTask),
        Box::new(TopicsMoveForwardTask),
    ]
}

/// Runs every registered task on `ctx`, collecting their events in order. A task
/// error short-circuits and propagates, leaving the caller's compensation to run.
/// Sequential `await?` over the boxed tasks: a plain `for` is the honest shape
/// here (no clean std combinator for fallible-async fold, and `domain` has no
/// `futures` dependency, so do NOT add one for this).
pub(crate) async fn run_coaching_session_hydration_tasks(
    ctx: &CoachingSessionHydrationContext<'_>,
) -> Result<Vec<DomainEvent>, Error> {
    let mut events = Vec::new();
    for task in coaching_session_hydration_tasks() {
        let produced = task.run(ctx).await?;
        debug!(
            "Hydration task '{}' produced {} event(s)",
            task.name(),
            produced.len()
        );
        events.extend(produced);
    }
    Ok(events)
}

/// What hydrating a new session carries into it, read without writing anything.
pub(crate) struct CarriedForward {
    /// The relationship's in-progress goals, capped as linking caps them.
    pub goals: Vec<goals::Model>,
    /// The prior session's Deferred topics, in their order.
    pub topics: Vec<coaching_session_topics::Model>,
}

/// What [`GoalsCarryForwardTask`] and [`TopicsMoveForwardTask`] will carry into a new session
/// of `coaching_relationship_id` at `date`, by the same reads, so its invite can list them
/// before they run.
pub(crate) async fn carried_forward(
    db: &impl ConnectionTrait,
    coaching_relationship_id: Id,
    date: NaiveDateTime,
) -> Result<CarriedForward, Error> {
    let goals =
        goal::find_in_progress_goals_by_coaching_relationship_id(db, coaching_relationship_id)
            .await?
            .into_iter()
            .take(goal::max_in_progress_goals())
            .collect();
    let topics =
        match coaching_session::find_prior_session(db, coaching_relationship_id, date).await? {
            Some(prior) => coaching_session_topic::find_by_coaching_session_id(db, prior.id)
                .await?
                .into_iter()
                .filter(|topic| topic.status == Status::Deferred)
                .collect(),
            None => Vec::new(),
        };
    Ok(CarriedForward { goals, topics })
}

/// Links the relationship's in-progress goals to the session and emits one
/// `CoachingSessionGoalCreated` per newly-linked goal. Behavior-identical to the
/// pre-registry inline path (same query, same events, same notify list).
struct GoalsCarryForwardTask;

#[async_trait::async_trait]
impl CoachingSessionHydrationTask for GoalsCarryForwardTask {
    fn name(&self) -> &'static str {
        "goals_carry_forward"
    }

    /// Reads as a sentence: link the in-progress goals, turn each linked id into a
    /// `CoachingSessionGoalCreated`, collect. The empty case falls out naturally
    /// (mapping an empty Vec yields no events), so no `is_empty` guard. The runner
    /// logs the produced count, so no intermediate debug binding here.
    async fn run(
        &self,
        ctx: &CoachingSessionHydrationContext<'_>,
    ) -> Result<Vec<DomainEvent>, Error> {
        Ok(coaching_session_goal::link_in_progress_goals_to_session(
            ctx.txn,
            ctx.session.coaching_relationship_id,
            ctx.session.id,
        )
        .await?
        .into_iter()
        .map(|goal_id| DomainEvent::CoachingSessionGoalCreated {
            coaching_relationship_id: ctx.relationship.id,
            coaching_session_id: ctx.session.id,
            goal_id,
            notify_user_ids: ctx.member_ids.to_vec(),
        })
        .collect())
    }
}

/// Moves the prior session's held Deferred topics into this session at hydration.
struct TopicsMoveForwardTask;

#[async_trait::async_trait]
impl CoachingSessionHydrationTask for TopicsMoveForwardTask {
    fn name(&self) -> &'static str {
        "topics_move_forward"
    }

    /// Reads as a sentence: find the prior session, move its held Deferred topics
    /// forward, and announce both sessions only when something actually moved.
    async fn run(
        &self,
        ctx: &CoachingSessionHydrationContext<'_>,
    ) -> Result<Vec<DomainEvent>, Error> {
        let Some(prior) = coaching_session::find_prior_session(
            ctx.txn,
            ctx.session.coaching_relationship_id,
            ctx.session.date,
        )
        .await?
        else {
            return Ok(Vec::new());
        };
        let moved =
            coaching_session_topic::move_deferred_to_session(ctx.txn, prior.id, ctx.session.id)
                .await?;
        if moved.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![
            DomainEvent::TopicsChanged {
                coaching_session_id: ctx.session.id,
                notify_user_ids: ctx.member_ids.to_vec(),
            },
            DomainEvent::TopicsChanged {
                coaching_session_id: prior.id,
                notify_user_ids: ctx.member_ids.to_vec(),
            },
        ])
    }
}
