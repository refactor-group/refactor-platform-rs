use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::goals::Model;
use crate::Id;
use entity_api::coaching_session_goal as CoachingSessionGoalApi;
use entity_api::coaching_sessions_goals;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{goal as GoalApi, goals, query};
use log::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};

pub use entity_api::goal::find_by_id;

pub async fn create(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    goal_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    let goal = GoalApi::create(&txn, goal_model, user_id).await?;
    link_to_created_in_session(&txn, &goal).await?;

    txn.commit().await.map_err(entity_api::error::Error::from)?;

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

/// Links a newly created goal to its `created_in_session` in the join table
/// so that "goals linked to session X" queries return it immediately.
async fn link_to_created_in_session(db: &impl ConnectionTrait, goal: &Model) -> Result<(), Error> {
    if let Some(session_id) = goal.created_in_session_id {
        CoachingSessionGoalApi::create(db, session_id, goal.id).await?;
        debug!(
            "Auto-linked goal {} to created-in session {}",
            goal.id, session_id
        );
    }
    Ok(())
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

// ── Coaching-session ↔ goal association (join table as implementation detail) ──

/// Links an existing goal to a coaching session.
pub async fn link_to_coaching_session(
    db: &DatabaseConnection,
    coaching_session_id: Id,
    goal_id: Id,
) -> Result<coaching_sessions_goals::Model, Error> {
    Ok(CoachingSessionGoalApi::create(db, coaching_session_id, goal_id).await?)
}

/// Unlinks a goal from a coaching session by the join-table record id.
pub async fn unlink_from_coaching_session(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    Ok(CoachingSessionGoalApi::delete_by_id(db, id).await?)
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

#[cfg(test)]
#[cfg(feature = "mock")]
mod integration_tests {
    use super::*;
    use entity_api::status::Status;
    use events::EventPublisher;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn create_test_goal_with(
        status: Status,
        title: Option<String>,
        coaching_relationship_id: Id,
    ) -> Model {
        let now = chrono::Utc::now().fixed_offset();
        Model {
            id: Id::new_v4(),
            coaching_relationship_id,
            created_in_session_id: None,
            user_id: Id::new_v4(),
            title,
            body: None,
            status,
            status_changed_at: None,
            completed_at: None,
            target_date: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn create_test_relationship(id: Id) -> crate::coaching_relationships::Model {
        let now = chrono::Utc::now().fixed_offset();
        crate::coaching_relationships::Model {
            id,
            organization_id: Id::new_v4(),
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            slug: "test-slug".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn create_publishes_event_on_success() {
        let relationship_id = Id::new_v4();
        let event_publisher = EventPublisher::new();

        let new_goal = create_test_goal_with(
            Status::NotStarted,
            Some("New goal".to_string()),
            relationship_id,
        );
        let relationship = create_test_relationship(relationship_id);

        // Mock sequence: goal save → relationship lookup
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![new_goal.clone()]])
            .append_query_results(vec![vec![relationship]])
            .into_connection();

        let result = create(&db, &event_publisher, new_goal, Id::new_v4()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn update_status_publishes_event_on_success() {
        let relationship_id = Id::new_v4();
        let event_publisher = EventPublisher::new();

        let current_goal = create_test_goal_with(
            Status::InProgress,
            Some("Already in-progress".to_string()),
            relationship_id,
        );
        let relationship = create_test_relationship(relationship_id);

        // Mock sequence: find_by_id → update_status save → relationship lookup
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![current_goal.clone()]])
            .append_query_results(vec![vec![current_goal.clone()]])
            .append_query_results(vec![vec![relationship]])
            .into_connection();

        let result = update_status(&db, &event_publisher, current_goal.id, Status::Completed).await;

        assert!(result.is_ok());
    }
}
