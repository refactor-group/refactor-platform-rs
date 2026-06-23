use crate::actions::Model;
use crate::coaching_session;
use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::Id;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::status::Status;
use entity_api::{actions, actions_user, query};
use log::*;
use sea_orm::DatabaseConnection;
use serde_json::Value;

// Mutations that emit SSE (create_with_assignees, update_with_assignees, update_status,
// delete_by_id) are wrapped below; the rest are direct re-exports.
pub use entity_api::action::{
    create, find_by_coaching_relationship, find_by_id, find_by_id_with_assignees, find_by_user,
    find_by_user_relationships, update, ActionWithAssignees, AssigneeFilter, AssigneeScope,
    CallerVisibility, FindByRelationshipParams, FindByUserParams, Scope,
};

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<actions::Column>,
{
    let actions = query::find_by::<actions::Entity, actions::Column, P>(db, params).await?;
    Ok(actions)
}

/// Finds actions with their assignee IDs.
///
/// This fetches actions matching the given parameters and includes
/// the assignee user IDs for each action.
pub async fn find_by_with_assignees<P>(
    db: &DatabaseConnection,
    params: P,
) -> Result<Vec<ActionWithAssignees>, Error>
where
    P: IntoQueryFilterMap + QuerySort<actions::Column>,
{
    let actions = query::find_by::<actions::Entity, actions::Column, P>(db, params).await?;

    // Batch fetch all assignees for all actions in one query (avoids N+1 issue)
    let action_ids = actions.iter().map(|a| a.id).collect();
    let mut assignees_map = actions_user::find_assignees_for_actions(db, action_ids).await?;

    // Build results with assignees from the map
    let mut result = Vec::with_capacity(actions.len());
    for action in actions {
        let assignee_ids = assignees_map.remove(&action.id).unwrap_or_default();
        result.push(ActionWithAssignees {
            action,
            assignee_ids,
        });
    }

    Ok(result)
}

/// Returns the user IDs currently assigned to the given action.
pub async fn find_assignee_ids(
    db: &DatabaseConnection,
    action_id: crate::Id,
) -> Result<Vec<crate::Id>, Error> {
    Ok(actions_user::find_user_ids_by_action_id(db, action_id).await?)
}

/// Best-effort SSE notify set for an action: the participants of the action's session. The DB
/// write is the contract; a failed lookup must NOT fail the mutation, so log and return None.
async fn action_notify_user_ids(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Option<Vec<Id>> {
    match coaching_session::find_participant_ids(db, coaching_session_id).await {
        Ok(ids) => Some(ids),
        Err(e) => {
            error!("action SSE: failed to resolve participants for session {coaching_session_id}: {e:?}");
            None
        }
    }
}

/// Publishes `ActionCreated` or `ActionUpdated` carrying the full action (with assignees).
async fn publish_action_changed(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    action: &ActionWithAssignees,
    created: bool,
) {
    let coaching_session_id = action.action.coaching_session_id;
    let Some(notify_user_ids) = action_notify_user_ids(db, coaching_session_id).await else {
        return;
    };
    let payload = serde_json::to_value(action).unwrap_or(Value::Null);
    let event = if created {
        DomainEvent::ActionCreated {
            coaching_session_id,
            action: payload,
            notify_user_ids,
        }
    } else {
        DomainEvent::ActionUpdated {
            coaching_session_id,
            action: payload,
            notify_user_ids,
        }
    };
    event_publisher.publish(event).await;
}

/// Creates an action (with optional assignees) and publishes `ActionCreated` to both participants.
pub async fn create_with_assignees(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    action_model: Model,
    user_id: Id,
    assignee_ids: Option<Vec<Id>>,
) -> Result<ActionWithAssignees, Error> {
    let action =
        entity_api::action::create_with_assignees(db, action_model, user_id, assignee_ids).await?;
    publish_action_changed(db, event_publisher, &action, true).await;
    Ok(action)
}

/// Updates an action (with optional assignee changes) and publishes `ActionUpdated`.
pub async fn update_with_assignees(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    model: Model,
    assignee_ids: Option<Vec<Id>>,
) -> Result<ActionWithAssignees, Error> {
    let action = entity_api::action::update_with_assignees(db, id, model, assignee_ids).await?;
    publish_action_changed(db, event_publisher, &action, false).await;
    Ok(action)
}

/// Updates an action's status and publishes `ActionUpdated` (with assignees re-read for the payload).
pub async fn update_status(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    status: Status,
) -> Result<Model, Error> {
    let action = entity_api::action::update_status(db, id, status).await?;
    if let Ok(with_assignees) = entity_api::action::find_by_id_with_assignees(db, id).await {
        publish_action_changed(db, event_publisher, &with_assignees, false).await;
    }
    Ok(action)
}

/// Deletes an action and publishes `ActionDeleted`. Captures the session id before deletion.
pub async fn delete_by_id(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
) -> Result<(), Error> {
    let coaching_session_id = entity_api::action::find_by_id(db, id)
        .await?
        .coaching_session_id;
    entity_api::action::delete_by_id(db, id).await?;
    if let Some(notify_user_ids) = action_notify_user_ids(db, coaching_session_id).await {
        event_publisher
            .publish(DomainEvent::ActionDeleted {
                coaching_session_id,
                action_id: id,
                notify_user_ids,
            })
            .await;
    }
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use crate::test_support::recording_publisher;
    use crate::{coaching_relationships, coaching_sessions};
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

    fn action_model(coaching_session_id: Id) -> Model {
        let now = chrono::Utc::now().fixed_offset();
        Model {
            id: Id::new_v4(),
            coaching_session_id,
            goal_id: None,
            user_id: Id::new_v4(),
            body: Some("Do the thing".to_string()),
            due_by: None,
            status: Status::default(),
            status_changed_at: now,
            created_at: now,
            updated_at: now,
        }
    }

    fn session_with_relationship(
        coaching_session_id: Id,
    ) -> (coaching_sessions::Model, coaching_relationships::Model) {
        let now = chrono::Utc::now().fixed_offset();
        let relationship_id = Id::new_v4();
        let session = coaching_sessions::Model {
            id: coaching_session_id,
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: None,
            collab_document_name: None,
            date: now.naive_utc(),
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now,
            updated_at: now,
            hydrated_at: None,
        };
        let relationship = coaching_relationships::Model {
            id: relationship_id,
            organization_id: Id::new_v4(),
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            slug: "test-slug".to_string(),
            created_at: now,
            updated_at: now,
        };
        (session, relationship)
    }

    fn assert_one_action_event(
        events: &[DomainEvent],
        expected_session_id: Id,
        expect_created: bool,
    ) {
        assert_eq!(events.len(), 1, "expected exactly one published event");
        match &events[0] {
            DomainEvent::ActionCreated {
                coaching_session_id,
                notify_user_ids,
                ..
            } if expect_created => {
                assert_eq!(*coaching_session_id, expected_session_id);
                assert_eq!(notify_user_ids.len(), 2, "coach + coachee");
            }
            DomainEvent::ActionUpdated {
                coaching_session_id,
                notify_user_ids,
                ..
            } if !expect_created => {
                assert_eq!(*coaching_session_id, expected_session_id);
                assert_eq!(notify_user_ids.len(), 2, "coach + coachee");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_with_assignees_publishes_action_created() {
        let session_id = Id::new_v4();
        let action = action_model(session_id);
        let (publisher, events) = recording_publisher();

        // create (INSERT RETURNING) → participant lookup (find_also_related). No assignee query (None).
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action.clone()]])
            .append_query_results(vec![vec![session_with_relationship(session_id)]])
            .into_connection();

        let result =
            create_with_assignees(&db, &publisher, action.clone(), action.user_id, None).await;

        assert!(result.is_ok());
        assert_one_action_event(&events.lock().unwrap(), session_id, true);
    }

    #[tokio::test]
    async fn update_status_publishes_action_updated() {
        let session_id = Id::new_v4();
        let action = action_model(session_id);
        let (publisher, events) = recording_publisher();

        // update_status: find_by_id → UPDATE RETURNING → find_by_id_with_assignees (find_by_id +
        // assignees query) → participant lookup.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action.clone()]])
            .append_query_results(vec![vec![action.clone()]])
            .append_query_results(vec![vec![action.clone()]])
            .append_query_results(vec![Vec::<entity::actions_users::Model>::new()])
            .append_query_results(vec![vec![session_with_relationship(session_id)]])
            .into_connection();

        let result = update_status(&db, &publisher, action.id, Status::default()).await;

        assert!(result.is_ok());
        assert_one_action_event(&events.lock().unwrap(), session_id, false);
    }

    #[tokio::test]
    async fn delete_by_id_publishes_action_deleted() {
        let session_id = Id::new_v4();
        let action = action_model(session_id);
        let (publisher, events) = recording_publisher();

        // delete: wrapper find_by_id → entity delete_by_id (find_by_id + DELETE) → participant lookup.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action.clone()]])
            .append_query_results(vec![vec![action.clone()]])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![session_with_relationship(session_id)]])
            .into_connection();

        let result = delete_by_id(&db, &publisher, action.id).await;

        assert!(result.is_ok());
        let recorded = events.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        match &recorded[0] {
            DomainEvent::ActionDeleted {
                coaching_session_id,
                action_id,
                notify_user_ids,
            } => {
                assert_eq!(*coaching_session_id, session_id);
                assert_eq!(*action_id, action.id);
                assert_eq!(notify_user_ids.len(), 2);
            }
            other => panic!("expected ActionDeleted, got {other:?}"),
        }
    }
}
