use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::goals::Model;
use crate::Id;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{coaching_session, goal as GoalApi, goals, query};
use log::*;
use sea_orm::DatabaseConnection;

pub use entity_api::goal::{find_by_coaching_session_id, find_by_id};

pub async fn create(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    goal_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    // Create the goal
    let goal = GoalApi::create(db, goal_model, user_id).await?;

    // Fetch the coaching session to get the relationship_id
    let coaching_session = coaching_session::find_by_id(db, goal.coaching_session_id).await?;

    // Fetch the coaching relationship to get the users to notify
    let relationship =
        crate::coaching_relationship::find_by_id(db, coaching_session.coaching_relationship_id)
            .await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    // Publish domain event
    event_publisher
        .publish(DomainEvent::GoalCreated {
            coaching_relationship_id: coaching_session.coaching_relationship_id,
            goal: serde_json::to_value(&goal).unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published GoalCreated event for goal {} in relationship {}",
        goal.id, coaching_session.coaching_relationship_id
    );

    Ok(goal)
}

pub async fn update(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    model: Model,
) -> Result<Model, Error> {
    // Update the goal
    let goal = GoalApi::update(db, id, model).await?;

    // Fetch the coaching session to get the relationship_id
    let coaching_session = coaching_session::find_by_id(db, goal.coaching_session_id).await?;

    // Fetch the coaching relationship to get the users to notify
    let relationship =
        crate::coaching_relationship::find_by_id(db, coaching_session.coaching_relationship_id)
            .await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    // Publish domain event
    event_publisher
        .publish(DomainEvent::GoalUpdated {
            coaching_relationship_id: coaching_session.coaching_relationship_id,
            goal: serde_json::to_value(&goal).unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published GoalUpdated event for goal {} in relationship {}",
        goal.id, coaching_session.coaching_relationship_id
    );

    Ok(goal)
}

pub async fn update_status(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    status: entity_api::status::Status,
) -> Result<Model, Error> {
    // Update the goal status
    let goal = GoalApi::update_status(db, id, status).await?;

    // Fetch the coaching session to get the relationship_id
    let coaching_session = coaching_session::find_by_id(db, goal.coaching_session_id).await?;

    // Fetch the coaching relationship to get the users to notify
    let relationship =
        crate::coaching_relationship::find_by_id(db, coaching_session.coaching_relationship_id)
            .await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    // Publish domain event
    event_publisher
        .publish(DomainEvent::GoalUpdated {
            coaching_relationship_id: coaching_session.coaching_relationship_id,
            goal: serde_json::to_value(&goal).unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published GoalUpdated event for goal {} in relationship {}",
        goal.id, coaching_session.coaching_relationship_id
    );

    Ok(goal)
}

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<goals::Column>,
{
    let goals = query::find_by::<goals::Entity, goals::Column, P>(db, params).await?;
    Ok(goals)
}
