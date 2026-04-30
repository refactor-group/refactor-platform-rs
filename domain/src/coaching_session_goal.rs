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
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};

/// Links an existing goal to a coaching session and publishes SSE events.
///
/// The link insert and any auto-promotion of the goal's status (from
/// `NotStarted`/`OnHold` to `InProgress`) happen atomically inside a transaction.
/// On commit, this publishes `CoachingSessionGoalCreated` and — when the link
/// promoted the goal — also `GoalUpdated` so subscribers see the new status.
pub async fn link_to_coaching_session(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    coaching_session_id: Id,
    goal_id: Id,
) -> Result<coaching_sessions_goals::Model, Error> {
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    let (link, promoted_goal) =
        CoachingSessionGoalApi::create(&txn, coaching_session_id, goal_id).await?;
    txn.commit().await.map_err(entity_api::error::Error::from)?;

    let (_, relationship) =
        crate::coaching_session::find_by_id_with_coaching_relationship(db, coaching_session_id)
            .await?;
    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];

    event_publisher
        .publish(DomainEvent::CoachingSessionGoalCreated {
            coaching_relationship_id: relationship.id,
            coaching_session_id,
            goal_id,
            notify_user_ids: notify_user_ids.clone(),
        })
        .await;

    debug!(
        "Published CoachingSessionGoalCreated event for goal {} in session {}",
        goal_id, coaching_session_id
    );

    if let Some(goal) = promoted_goal {
        event_publisher
            .publish(DomainEvent::GoalUpdated {
                coaching_relationship_id: goal.coaching_relationship_id,
                goal: serde_json::to_value(&goal).unwrap_or(serde_json::Value::Null),
                notify_user_ids,
            })
            .await;

        debug!(
            "Published GoalUpdated event for promoted goal {} in relationship {}",
            goal.id, goal.coaching_relationship_id
        );
    }

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

#[cfg(test)]
#[cfg(feature = "mock")]
mod integration_tests {
    use super::*;
    use async_trait::async_trait;
    use entity_api::coaching_relationships;
    use entity_api::coaching_sessions;
    use entity_api::status::Status;
    use events::EventHandler;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use std::sync::{Arc, Mutex};

    /// Recording event handler — captures every published event in order so
    /// tests can assert exactly which events fired and in what sequence.
    struct RecordingHandler {
        events: Arc<Mutex<Vec<DomainEvent>>>,
    }

    #[async_trait]
    impl EventHandler for RecordingHandler {
        async fn handle(&self, event: &DomainEvent) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    fn recording_publisher() -> (EventPublisher, Arc<Mutex<Vec<DomainEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(RecordingHandler {
            events: events.clone(),
        });
        let publisher = EventPublisher::new().with_handler(handler);
        (publisher, events)
    }

    fn build_goal(status: Status, relationship_id: Id) -> Model {
        let now = chrono::Utc::now().fixed_offset();
        Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            created_in_session_id: None,
            user_id: Id::new_v4(),
            title: Some("Test goal".to_string()),
            body: None,
            status,
            status_changed_at: None,
            completed_at: None,
            target_date: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn build_relationship(id: Id) -> coaching_relationships::Model {
        let now = chrono::Utc::now().fixed_offset();
        coaching_relationships::Model {
            id,
            organization_id: Id::new_v4(),
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            slug: "test-rel".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    fn build_session(id: Id, relationship_id: Id) -> coaching_sessions::Model {
        let now = chrono::Utc::now();
        coaching_sessions::Model {
            id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: now.naive_utc(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn build_link(session_id: Id, goal_id: Id) -> coaching_sessions_goals::Model {
        let now = chrono::Utc::now().fixed_offset();
        coaching_sessions_goals::Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id,
            created_at: now,
            updated_at: now,
        }
    }

    /// Self-heal scenario: a goal was linked to a session pre-invariant and
    /// is sitting in `NotStarted`. The coach manually re-links it to a new
    /// session — BE auto-promotes to `InProgress` and publishes both
    /// `CoachingSessionGoalCreated` and `GoalUpdated` in that order.
    #[tokio::test]
    async fn link_self_heals_legacy_not_started_goal_and_publishes_both_events() {
        let relationship_id = Id::new_v4();
        let new_session_id = Id::new_v4();
        let goal = build_goal(Status::NotStarted, relationship_id);
        let promoted = Model {
            status: Status::InProgress,
            ..goal.clone()
        };
        let link = build_link(new_session_id, goal.id);
        let relationship = build_relationship(relationship_id);

        // Mock sequence (inside the txn opened by link_to_coaching_session):
        //   1. SELECT goal by id (entity_api::coaching_session_goal::create)
        //   2. SELECT existing link (duplicate-check, returns empty)
        //   3. SELECT in-progress goals on relationship (cap check, returns empty)
        //   4. INSERT into coaching_sessions_goals (the new link row)
        //   5. UPDATE goals (auto-promotion to InProgress)
        // After commit:
        //   6. find_by_id_with_coaching_relationship for event-routing user IDs
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal.clone()]])
            .append_query_results(vec![Vec::<coaching_sessions_goals::Model>::new()])
            .append_query_results(vec![Vec::<Model>::new()])
            .append_query_results(vec![vec![link.clone()]])
            .append_query_results(vec![vec![promoted.clone()]])
            // The find_by_id_with_coaching_relationship query is a JOIN that
            // returns (link, relationship) — the mock harness for find_also_linked
            // expects a row of the joined shape.
            .append_query_results(vec![vec![(
                build_session(new_session_id, relationship_id),
                Some(relationship.clone()),
            )]])
            .into_connection();

        let (publisher, recorded) = recording_publisher();
        let result = link_to_coaching_session(&db, &publisher, new_session_id, goal.id).await;

        assert!(
            result.is_ok(),
            "self-heal link should succeed: {:?}",
            result.err()
        );

        let events = recorded.lock().unwrap();
        assert_eq!(
            events.len(),
            2,
            "expected both events to fire, got {events:?}"
        );

        // CoachingSessionGoalCreated must fire first (link side effect).
        match &events[0] {
            DomainEvent::CoachingSessionGoalCreated {
                coaching_session_id,
                goal_id: ev_goal_id,
                ..
            } => {
                assert_eq!(*coaching_session_id, new_session_id);
                assert_eq!(*ev_goal_id, goal.id);
            }
            other => panic!("expected CoachingSessionGoalCreated first, got {other:?}"),
        }

        // GoalUpdated must fire second, carrying the promoted goal entity.
        match &events[1] {
            DomainEvent::GoalUpdated {
                coaching_relationship_id,
                goal: goal_value,
                ..
            } => {
                assert_eq!(*coaching_relationship_id, relationship_id);
                let status_str = goal_value
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                // Status serializes as PascalCase on the wire (matches the
                // RelationshipGoalProgress v4 contract — DB stores snake_case
                // but serde wire format is PascalCase).
                assert_eq!(
                    status_str, "InProgress",
                    "promoted goal in event payload should serialize status as InProgress"
                );
            }
            other => panic!("expected GoalUpdated second, got {other:?}"),
        }
    }

    /// Linking a goal that is already `InProgress` only fires the link event,
    /// not `GoalUpdated` — confirms we don't publish a redundant status event
    /// when no promotion actually happened.
    #[tokio::test]
    async fn link_in_progress_goal_only_fires_link_event() {
        let relationship_id = Id::new_v4();
        let new_session_id = Id::new_v4();
        let goal = build_goal(Status::InProgress, relationship_id);
        let link = build_link(new_session_id, goal.id);
        let relationship = build_relationship(relationship_id);

        // Mock sequence (no cap-check, no promotion update):
        //   1. SELECT goal by id
        //   2. SELECT existing link (duplicate-check, returns empty)
        //   3. INSERT into coaching_sessions_goals
        //   4. find_by_id_with_coaching_relationship
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal.clone()]])
            .append_query_results(vec![Vec::<coaching_sessions_goals::Model>::new()])
            .append_query_results(vec![vec![link.clone()]])
            .append_query_results(vec![vec![(
                build_session(new_session_id, relationship_id),
                Some(relationship.clone()),
            )]])
            .into_connection();

        let (publisher, recorded) = recording_publisher();
        let result = link_to_coaching_session(&db, &publisher, new_session_id, goal.id).await;
        assert!(result.is_ok(), "link should succeed: {:?}", result.err());

        let events = recorded.lock().unwrap();
        assert_eq!(
            events.len(),
            1,
            "expected only CoachingSessionGoalCreated to fire, got {events:?}"
        );
        assert!(
            matches!(events[0], DomainEvent::CoachingSessionGoalCreated { .. }),
            "expected CoachingSessionGoalCreated, got {:?}",
            events[0]
        );
    }
}
