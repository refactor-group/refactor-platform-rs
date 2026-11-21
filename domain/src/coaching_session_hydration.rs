use crate::coaching_relationships;
use crate::coaching_sessions::Model;
use crate::error::Error;
use crate::events::DomainEvent;
use crate::{coaching_session_topics, goals, Id};
use entity_api::coaching_session;
use entity_api::coaching_session_goal;
use entity_api::coaching_session_topic;
use log::*;
use sea_orm::DatabaseTransaction;

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

/// What hydration carried into a session, and the events to publish after commit.
#[derive(Default)]
pub(crate) struct Hydrated {
    pub events: Vec<DomainEvent>,
    /// The goals newly linked to the session.
    pub goals: Vec<goals::Model>,
    /// The topics moved into the session, in their order.
    pub topics: Vec<coaching_session_topics::Model>,
}

impl Hydrated {
    /// Appends what `other` carried after what this holds.
    fn extend(&mut self, other: Hydrated) {
        self.events.extend(other.events);
        self.goals.extend(other.goals);
        self.topics.extend(other.topics);
    }
}

/// A unit of deferred, prerequisite work run at-latest on a coaching session's
/// first read (or eagerly at create), inside the hydration transaction. Loosely
/// coupled: registering one requires no change to `create`/`ensure_hydrated`.
#[async_trait::async_trait]
pub(crate) trait CoachingSessionHydrationTask: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self, ctx: &CoachingSessionHydrationContext<'_>) -> Result<Hydrated, Error>;
}

/// Ordered registry of hydration tasks.
fn coaching_session_hydration_tasks() -> Vec<Box<dyn CoachingSessionHydrationTask>> {
    vec![
        Box::new(GoalsCarryForwardTask),
        Box::new(TopicsMoveForwardTask),
    ]
}

/// Runs every registered task on `ctx`, collecting what they carried and their events in
/// order. A task error short-circuits and propagates, leaving the caller's compensation to
/// run. Sequential `await?` over the boxed tasks: each runs on the same transaction.
pub(crate) async fn run_coaching_session_hydration_tasks(
    ctx: &CoachingSessionHydrationContext<'_>,
) -> Result<Hydrated, Error> {
    let mut hydrated = Hydrated::default();
    for task in coaching_session_hydration_tasks() {
        let produced = task.run(ctx).await?;
        debug!(
            "Hydration task '{}' produced {} event(s)",
            task.name(),
            produced.events.len()
        );
        hydrated.extend(produced);
    }
    Ok(hydrated)
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

    /// Reads as a sentence: link the in-progress goals, and announce each one linked. The
    /// empty case falls out naturally (mapping an empty Vec yields no events).
    async fn run(&self, ctx: &CoachingSessionHydrationContext<'_>) -> Result<Hydrated, Error> {
        let goals = coaching_session_goal::link_in_progress_goals_to_session(
            ctx.txn,
            ctx.session.coaching_relationship_id,
            ctx.session.id,
        )
        .await?;
        Ok(Hydrated {
            events: goals
                .iter()
                .map(|goal| DomainEvent::CoachingSessionGoalCreated {
                    coaching_relationship_id: ctx.relationship.id,
                    coaching_session_id: ctx.session.id,
                    goal_id: goal.id,
                    notify_user_ids: ctx.member_ids.to_vec(),
                })
                .collect(),
            goals,
            ..Hydrated::default()
        })
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
    async fn run(&self, ctx: &CoachingSessionHydrationContext<'_>) -> Result<Hydrated, Error> {
        let Some(prior) = coaching_session::find_prior_session(
            ctx.txn,
            ctx.session.coaching_relationship_id,
            ctx.session.date,
        )
        .await?
        else {
            return Ok(Hydrated::default());
        };
        let topics =
            coaching_session_topic::move_deferred_to_session(ctx.txn, prior.id, ctx.session.id)
                .await?;
        if topics.is_empty() {
            return Ok(Hydrated::default());
        }
        Ok(Hydrated {
            events: [ctx.session.id, prior.id]
                .map(|coaching_session_id| DomainEvent::TopicsChanged {
                    coaching_session_id,
                    notify_user_ids: ctx.member_ids.to_vec(),
                })
                .to_vec(),
            topics,
            ..Hydrated::default()
        })
    }
}
