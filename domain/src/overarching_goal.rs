use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::overarching_goals::Model;
use crate::Id;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{
    coaching_session, overarching_goal as OverarchingGoalApi, overarching_goals, query,
};
use log::*;
use sea_orm::DatabaseConnection;

pub use entity_api::overarching_goal::{find_by_coaching_session_id, find_by_id};

pub async fn create(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    overarching_goal_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    // Create the overarching goal
    let overarching_goal = OverarchingGoalApi::create(db, overarching_goal_model, user_id).await?;

    // Fetch the coaching session to get the relationship_id
    let coaching_session =
        coaching_session::find_by_id(db, overarching_goal.coaching_session_id).await?;

    // Fetch the coaching relationship to get the users to notify
    let relationship =
        crate::coaching_relationship::find_by_id(db, coaching_session.coaching_relationship_id)
            .await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    // Publish domain event
    event_publisher
        .publish(DomainEvent::OverarchingGoalCreated {
            coaching_relationship_id: coaching_session.coaching_relationship_id,
            overarching_goal: serde_json::to_value(&overarching_goal)
                .unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published OverarchingGoalCreated event for goal {} in relationship {}",
        overarching_goal.id, coaching_session.coaching_relationship_id
    );

    Ok(overarching_goal)
}

pub async fn update(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    model: Model,
) -> Result<Model, Error> {
    // Update the overarching goal
    let overarching_goal = OverarchingGoalApi::update(db, id, model).await?;

    // Fetch the coaching session to get the relationship_id
    let coaching_session =
        coaching_session::find_by_id(db, overarching_goal.coaching_session_id).await?;

    // Fetch the coaching relationship to get the users to notify
    let relationship =
        crate::coaching_relationship::find_by_id(db, coaching_session.coaching_relationship_id)
            .await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    // Publish domain event
    event_publisher
        .publish(DomainEvent::OverarchingGoalUpdated {
            coaching_relationship_id: coaching_session.coaching_relationship_id,
            overarching_goal: serde_json::to_value(&overarching_goal)
                .unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published OverarchingGoalUpdated event for goal {} in relationship {}",
        overarching_goal.id, coaching_session.coaching_relationship_id
    );

    Ok(overarching_goal)
}

pub async fn update_status(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    status: entity_api::status::Status,
) -> Result<Model, Error> {
    // Update the overarching goal status
    let overarching_goal = OverarchingGoalApi::update_status(db, id, status).await?;

    // Fetch the coaching session to get the relationship_id
    let coaching_session =
        coaching_session::find_by_id(db, overarching_goal.coaching_session_id).await?;

    // Fetch the coaching relationship to get the users to notify
    let relationship =
        crate::coaching_relationship::find_by_id(db, coaching_session.coaching_relationship_id)
            .await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    // Publish domain event
    event_publisher
        .publish(DomainEvent::OverarchingGoalUpdated {
            coaching_relationship_id: coaching_session.coaching_relationship_id,
            overarching_goal: serde_json::to_value(&overarching_goal)
                .unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published OverarchingGoalUpdated event for goal {} in relationship {}",
        overarching_goal.id, coaching_session.coaching_relationship_id
    );

    Ok(overarching_goal)
}

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<overarching_goals::Column>,
{
    let overarching_goals =
        query::find_by::<overarching_goals::Entity, overarching_goals::Column, P>(db, params)
            .await?;
    Ok(overarching_goals)
}
