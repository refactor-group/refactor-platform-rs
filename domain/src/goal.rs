use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::goals::Model;
use crate::Id;
use entity_api::coaching_session_goal as CoachingSessionGoalApi;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{goal as GoalApi, goals, query};
use log::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};

pub use entity_api::goal::find_by_id;

// Re-export coaching-session ↔ goal join operations so the web layer
// interacts with goals as a single domain concept rather than knowing
// about the join table as a separate module.
pub use crate::coaching_session_goal::{
    find_coaching_sessions_by_goal_id, find_goals_by_coaching_session_id,
    find_in_progress_goals_by_coaching_session_id, link_to_coaching_session,
    unlink_from_coaching_session, unlink_goal_from_coaching_session,
};

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

    let notify_user_ids =
        find_notify_user_ids_for_relationship(db, goal.coaching_relationship_id).await?;

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
    publish_goal_updated(db, event_publisher, &goal).await?;
    Ok(goal)
}

pub async fn update_status(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    status: entity_api::status::Status,
) -> Result<Model, Error> {
    let goal = GoalApi::update_status(db, id, status).await?;
    publish_goal_updated(db, event_publisher, &goal).await?;
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

    let notify_user_ids =
        find_notify_user_ids_for_relationship(db, goal.coaching_relationship_id).await?;

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

// ── Event publishing helpers ─────────────────────────────────────────

/// Looks up the coaching relationship and returns the user IDs that should
/// receive SSE notifications (coach + coachee).
async fn find_notify_user_ids_for_relationship(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<Vec<Id>, Error> {
    let relationship =
        crate::coaching_relationship::find_by_id(db, coaching_relationship_id).await?;
    Ok(vec![relationship.coach_id, relationship.coachee_id])
}

/// Publishes a `GoalUpdated` SSE event. Shared by `update` and `update_status`.
async fn publish_goal_updated(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    goal: &Model,
) -> Result<(), Error> {
    let notify_user_ids =
        find_notify_user_ids_for_relationship(db, goal.coaching_relationship_id).await?;

    event_publisher
        .publish(DomainEvent::GoalUpdated {
            coaching_relationship_id: goal.coaching_relationship_id,
            goal: serde_json::to_value(goal).unwrap_or(serde_json::Value::Null),
            notify_user_ids,
        })
        .await;

    debug!(
        "Published GoalUpdated event for goal {} in relationship {}",
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

#[cfg(test)]
#[cfg(feature = "mock")]
mod integration_tests {
    use super::*;
    use entity_api::coaching_sessions_goals;
    use entity_api::status::Status;
    use events::EventPublisher;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn create_test_goal_with(
        status: Status,
        title: Option<String>,
        coaching_relationship_id: Id,
    ) -> Model {
        create_test_goal(status, title, coaching_relationship_id, None)
    }

    fn create_test_goal(
        status: Status,
        title: Option<String>,
        coaching_relationship_id: Id,
        created_in_session_id: Option<Id>,
    ) -> Model {
        let now = chrono::Utc::now().fixed_offset();
        Model {
            id: Id::new_v4(),
            coaching_relationship_id,
            created_in_session_id,
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

        // Mock sequence (inside txn): goal save → (no session link) → relationship lookup
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![new_goal.clone()]])
            .append_query_results(vec![vec![relationship]])
            .into_connection();

        let result = create(&db, &event_publisher, new_goal, Id::new_v4()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn create_with_session_links_to_join_table() {
        let relationship_id = Id::new_v4();
        let session_id = Id::new_v4();
        let event_publisher = EventPublisher::new();

        let new_goal = create_test_goal(
            Status::NotStarted,
            Some("Session-linked goal".to_string()),
            relationship_id,
            Some(session_id),
        );
        let relationship = create_test_relationship(relationship_id);

        let now = chrono::Utc::now().fixed_offset();
        let join_row = coaching_sessions_goals::Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id: new_goal.id,
            created_at: now,
            updated_at: now,
        };

        // Mock sequence (inside txn): goal save → join table save → relationship lookup
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![new_goal.clone()]])
            .append_query_results(vec![vec![join_row]])
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
