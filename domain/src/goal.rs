use crate::error::{Error, InternalErrorKind};
use crate::events::{DomainEvent, EventPublisher};
use crate::goals::Model;
use crate::status::Status;
use crate::Id;
use entity_api::coaching_session_goal as CoachingSessionGoalApi;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{goal as GoalApi, goals, query};
use log::*;
use sea_orm::DatabaseConnection;

pub use entity_api::goal::find_by_id;

/// Maximum number of active (`InProgress`) goals allowed per coaching relationship.
pub const MAX_ACTIVE_GOALS: usize = 3;

/// Lightweight projection of a goal for use in the `ActiveGoalLimitReached` error.
/// Carries just enough info for the frontend to present a "swap" dialog.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct GoalSummary {
    pub id: Id,
    pub title: String,
}

pub async fn create(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    goal_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    if goal_model.status == Status::InProgress {
        check_active_goal_limit(db, goal_model.coaching_relationship_id).await?;
    }

    let goal = GoalApi::create(db, goal_model, user_id).await?;

    // CHANGEME: Remove when carry-forward workflow (PR3) replaces auto-linking
    link_to_originating_session(db, &goal).await?;

    let relationship =
        crate::coaching_relationship::find_by_id(db, goal.coaching_relationship_id).await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    event_publisher
        .publish(DomainEvent::GoalCreated {
            coaching_relationship_id: goal.coaching_relationship_id,
            goal: serde_json::to_value(&goal).unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published GoalCreated event for goal {} in relationship {}",
        goal.id, goal.coaching_relationship_id
    );

    Ok(goal)
}

/// If the goal was created within a session context, automatically link it
/// to that session in the coaching_sessions_goals join table.
///
/// CHANGEME: Remove this when the full goals rework carry-forward workflow
/// is in place (PR3). At that point, coaching_sessions_goals rows will be
/// created at session-creation time and the frontend will manage linking
/// explicitly via POST /coaching_session_goals.
async fn link_to_originating_session(db: &DatabaseConnection, goal: &Model) -> Result<(), Error> {
    if let Some(session_id) = goal.created_in_session_id {
        debug!(
            "Auto-linking goal {} to originating session {}",
            goal.id, session_id
        );
        CoachingSessionGoalApi::create(db, session_id, goal.id).await?;
    }
    Ok(())
}

pub async fn update(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    model: Model,
) -> Result<Model, Error> {
    // Check active goal limit if the update would transition to InProgress.
    if model.status == Status::InProgress {
        let current_goal = GoalApi::find_by_id(db, id).await?;
        if current_goal.status != Status::InProgress {
            check_active_goal_limit(db, current_goal.coaching_relationship_id).await?;
        }
    }

    let goal = GoalApi::update(db, id, model).await?;

    let relationship =
        crate::coaching_relationship::find_by_id(db, goal.coaching_relationship_id).await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    event_publisher
        .publish(DomainEvent::GoalUpdated {
            coaching_relationship_id: goal.coaching_relationship_id,
            goal: serde_json::to_value(&goal).unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published GoalUpdated event for goal {} in relationship {}",
        goal.id, goal.coaching_relationship_id
    );

    Ok(goal)
}

pub async fn update_status(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    status: entity_api::status::Status,
) -> Result<Model, Error> {
    // Only check the limit when transitioning TO InProgress from a non-InProgress status.
    if status == Status::InProgress {
        let current_goal = GoalApi::find_by_id(db, id).await?;
        if current_goal.status != Status::InProgress {
            check_active_goal_limit(db, current_goal.coaching_relationship_id).await?;
        }
    }

    let goal = GoalApi::update_status(db, id, status).await?;

    let relationship =
        crate::coaching_relationship::find_by_id(db, goal.coaching_relationship_id).await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    event_publisher
        .publish(DomainEvent::GoalUpdated {
            coaching_relationship_id: goal.coaching_relationship_id,
            goal: serde_json::to_value(&goal).unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published GoalUpdated event for goal {} in relationship {}",
        goal.id, goal.coaching_relationship_id
    );

    Ok(goal)
}

/// Deletes a goal by id and publishes a GoalDeleted domain event.
pub async fn delete(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
) -> Result<(), Error> {
    // delete_by_id returns the model before deletion so we can publish the event
    let goal = GoalApi::delete_by_id(db, id).await?;

    let relationship =
        crate::coaching_relationship::find_by_id(db, goal.coaching_relationship_id).await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    event_publisher
        .publish(DomainEvent::GoalDeleted {
            coaching_relationship_id: goal.coaching_relationship_id,
            goal_id: goal.id,
            notify_user_ids,
        })
        .await;

    debug!(
        "Published GoalDeleted event for goal {} in relationship {}",
        goal.id, goal.coaching_relationship_id
    );

    Ok(())
}

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<goals::Column>,
{
    let goals = query::find_by::<goals::Entity, goals::Column, P>(db, params).await?;
    Ok(goals)
}

impl From<Model> for GoalSummary {
    fn from(goal: Model) -> Self {
        Self {
            id: goal.id,
            title: goal.title.unwrap_or_default(),
        }
    }
}

impl GoalSummary {
    /// Converts a list of goal models into summaries, optionally excluding one goal by id.
    pub fn from_goals(goals: Vec<Model>, exclude_id: Option<Id>) -> Vec<Self> {
        goals
            .into_iter()
            .filter(|g| exclude_id != Some(g.id))
            .map(Self::from)
            .collect()
    }
}

impl Error {
    fn active_goal_limit_reached(active_goals: Vec<GoalSummary>) -> Self {
        Self {
            source: None,
            error_kind: crate::error::DomainErrorKind::Internal(
                InternalErrorKind::ActiveGoalLimitReached { active_goals },
            ),
        }
    }
}

/// Checks that adding one more `InProgress` goal to a coaching relationship
/// would not exceed `MAX_ACTIVE_GOALS`. If it would, returns an
/// `ActiveGoalLimitReached` error carrying summaries of the current active goals.
async fn check_active_goal_limit(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<(), Error> {
    let active_goals =
        GoalApi::find_active_goals_by_coaching_relationship_id(db, coaching_relationship_id)
            .await?;

    if active_goals.len() >= MAX_ACTIVE_GOALS {
        return Err(Error::active_goal_limit_reached(GoalSummary::from_goals(
            active_goals,
            None,
        )));
    }

    Ok(())
}
