//! Domain logic for coaching-session ↔ goal associations (join table).
//!
//! Handles linking/unlinking goals to coaching sessions and publishes
//! SSE events for real-time UI updates.

use std::collections::HashMap;

use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::goals::Model;
use crate::Id;
use entity_api::coaching_session_goal as CoachingSessionGoalApi;
use entity_api::coaching_sessions_goals;
use log::*;
use sea_orm::{ConnectionTrait, DatabaseConnection};

/// Links an existing goal to a coaching session and publishes an SSE event.
pub async fn link_to_coaching_session(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    coaching_session_id: Id,
    goal_id: Id,
) -> Result<coaching_sessions_goals::Model, Error> {
    let link = CoachingSessionGoalApi::create(db, coaching_session_id, goal_id).await?;

    let (_, relationship) =
        crate::coaching_session::find_by_id_with_coaching_relationship(db, coaching_session_id)
            .await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    event_publisher
        .publish(DomainEvent::CoachingSessionGoalCreated {
            coaching_relationship_id: relationship.id,
            coaching_session_id,
            goal_id,
            notify_user_ids,
        })
        .await;

    debug!(
        "Published CoachingSessionGoalCreated event for goal {} in session {}",
        goal_id, coaching_session_id
    );

    Ok(link)
}

/// Unlinks a goal from a coaching session by the join-table record id
/// and publishes an SSE event.
pub async fn unlink_from_coaching_session(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
) -> Result<(), Error> {
    // Single query: join table record + relationship (via two JOINs)
    let (link, relationship) =
        CoachingSessionGoalApi::find_by_id_with_coaching_relationship(db, id).await?;

    CoachingSessionGoalApi::delete_by_id(db, id).await?;

    publish_session_goal_deleted(event_publisher, &link, &relationship).await;

    Ok(())
}

/// Unlinks a goal from a coaching session by the (coaching_session_id, goal_id) pair
/// and publishes an SSE event.
pub async fn unlink_goal_from_coaching_session(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    coaching_session_id: Id,
    goal_id: Id,
) -> Result<(), Error> {
    let (link, relationship) =
        CoachingSessionGoalApi::find_by_session_and_goal_with_coaching_relationship(
            db,
            coaching_session_id,
            goal_id,
        )
        .await?;

    CoachingSessionGoalApi::delete_by_id(db, link.id).await?;

    publish_session_goal_deleted(event_publisher, &link, &relationship).await;

    Ok(())
}

/// Returns all goal models linked to a coaching session (eager-loaded).
pub async fn find_goals_by_coaching_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(CoachingSessionGoalApi::find_goals_by_coaching_session_id(db, coaching_session_id).await?)
}

/// Returns up to the maximum allowed in-progress goals linked to a coaching session.
pub async fn find_in_progress_goals_by_coaching_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(
        CoachingSessionGoalApi::find_in_progress_goals_by_coaching_session_id(
            db,
            coaching_session_id,
        )
        .await?,
    )
}

/// Returns all join-table records for a given goal (sessions linked to it).
pub async fn find_coaching_sessions_by_goal_id(
    db: &DatabaseConnection,
    goal_id: Id,
) -> Result<Vec<coaching_sessions_goals::Model>, Error> {
    Ok(CoachingSessionGoalApi::find_by_goal_id(db, goal_id).await?)
}

/// Returns all goals for multiple sessions, grouped by session ID.
///
/// When `session_ids` is provided directly, queries goals for those sessions.
/// When `coaching_relationship_id` is provided, first resolves all session IDs
/// for that relationship, then batch-loads their goals.
pub async fn find_goals_grouped_by_session_ids(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, Vec<Model>>, Error> {
    Ok(CoachingSessionGoalApi::find_goals_grouped_by_session_ids(db, session_ids).await?)
}

/// Returns all session IDs belonging to a coaching relationship.
pub async fn find_session_ids_by_coaching_relationship_id(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<Vec<Id>, Error> {
    Ok(
        CoachingSessionGoalApi::find_session_ids_by_coaching_relationship_id(
            db,
            coaching_relationship_id,
        )
        .await?,
    )
}

// ── Event publishing helpers ─────────────────────────────────────────

/// Publishes a `CoachingSessionGoalDeleted` SSE event. Shared by both
/// unlink-by-id and unlink-by-session-and-goal paths.
async fn publish_session_goal_deleted(
    event_publisher: &EventPublisher,
    link: &coaching_sessions_goals::Model,
    relationship: &entity_api::coaching_relationships::Model,
) {
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    event_publisher
        .publish(DomainEvent::CoachingSessionGoalDeleted {
            coaching_relationship_id: relationship.id,
            coaching_session_id: link.coaching_session_id,
            goal_id: link.goal_id,
            notify_user_ids,
        })
        .await;

    debug!(
        "Published CoachingSessionGoalDeleted event for goal {} in session {}",
        link.goal_id, link.coaching_session_id
    );
}
