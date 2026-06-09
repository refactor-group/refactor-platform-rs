use crate::coaching_relationships;
use crate::coaching_sessions::Model;
use crate::error::Error;
use crate::events::DomainEvent;
use entity_api::coaching_session_goal;
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

/// Ordered registry of hydration tasks. Goals-only for now.
fn coaching_session_hydration_tasks() -> Vec<Box<dyn CoachingSessionHydrationTask>> {
    vec![Box::new(GoalsCarryForwardTask)]
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
        let notify_user_ids = vec![ctx.relationship.coach_id, ctx.relationship.coachee_id];
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
            notify_user_ids: notify_user_ids.clone(),
        })
        .collect())
    }
}
