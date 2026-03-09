use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::goals::Model;
use crate::Id;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{goal as GoalApi, goals, query};
use log::*;
use sea_orm::DatabaseConnection;

pub use entity_api::goal::find_by_id;

pub async fn create(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    goal_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    let goal = GoalApi::create(db, goal_model, user_id).await?;

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

pub async fn update(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    model: Model,
) -> Result<Model, Error> {
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
