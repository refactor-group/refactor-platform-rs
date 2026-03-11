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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_goal(status: Status, title: Option<String>) -> Model {
        let now = chrono::Utc::now().fixed_offset();
        Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
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

    #[test]
    fn goal_summary_from_model_with_title() {
        let goal = create_test_goal(Status::InProgress, Some("My Goal".to_string()));
        let summary = GoalSummary::from(goal.clone());

        assert_eq!(summary.id, goal.id);
        assert_eq!(summary.title, "My Goal");
    }

    #[test]
    fn goal_summary_from_model_with_none_title_defaults_to_empty() {
        let goal = create_test_goal(Status::InProgress, None);
        let summary = GoalSummary::from(goal);

        assert_eq!(summary.title, "");
    }

    #[test]
    fn from_goals_returns_all_when_no_exclusion() {
        let goals = vec![
            create_test_goal(Status::InProgress, Some("A".to_string())),
            create_test_goal(Status::InProgress, Some("B".to_string())),
        ];

        let summaries = GoalSummary::from_goals(goals, None);

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].title, "A");
        assert_eq!(summaries[1].title, "B");
    }

    #[test]
    fn from_goals_excludes_by_id() {
        let goal_a = create_test_goal(Status::InProgress, Some("A".to_string()));
        let goal_b = create_test_goal(Status::InProgress, Some("B".to_string()));
        let exclude = goal_a.id;

        let summaries = GoalSummary::from_goals(vec![goal_a, goal_b], Some(exclude));

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].title, "B");
    }

    #[test]
    fn from_goals_empty_input_returns_empty() {
        let summaries = GoalSummary::from_goals(vec![], None);
        assert!(summaries.is_empty());
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod integration_tests {
    use super::*;
    use crate::error::DomainErrorKind;
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
    async fn create_rejects_in_progress_when_at_limit() {
        let relationship_id = Id::new_v4();
        let event_publisher = EventPublisher::new();

        // Mock: find_active_goals returns MAX_ACTIVE_GOALS InProgress goals → limit hit
        let active_goals: Vec<Model> = (0..MAX_ACTIVE_GOALS)
            .map(|i| {
                create_test_goal_with(
                    Status::InProgress,
                    Some(format!("Goal {i}")),
                    relationship_id,
                )
            })
            .collect();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![active_goals])
            .into_connection();

        let new_goal = create_test_goal_with(
            Status::InProgress,
            Some("One too many".to_string()),
            relationship_id,
        );

        let result = create(&db, &event_publisher, new_goal, Id::new_v4()).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::ActiveGoalLimitReached {
                ref active_goals
            }) if active_goals.len() == MAX_ACTIVE_GOALS
        ));
    }

    #[tokio::test]
    async fn create_allows_in_progress_when_under_limit() {
        let relationship_id = Id::new_v4();
        let event_publisher = EventPublisher::new();

        let active_goals: Vec<Model> = (0..MAX_ACTIVE_GOALS - 1)
            .map(|i| {
                create_test_goal_with(
                    Status::InProgress,
                    Some(format!("Goal {i}")),
                    relationship_id,
                )
            })
            .collect();

        let new_goal = create_test_goal_with(
            Status::InProgress,
            Some("Fits under limit".to_string()),
            relationship_id,
        );
        let relationship = create_test_relationship(relationship_id);

        // Mock sequence: active goals query → goal save → relationship lookup
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![active_goals])
            .append_query_results(vec![vec![new_goal.clone()]])
            .append_query_results(vec![vec![relationship]])
            .into_connection();

        let result = create(&db, &event_publisher, new_goal, Id::new_v4()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn create_allows_not_started_even_at_limit() {
        let relationship_id = Id::new_v4();
        let event_publisher = EventPublisher::new();

        // NotStarted bypasses the limit check entirely — no active goals query needed
        let new_goal = create_test_goal_with(
            Status::NotStarted,
            Some("Queued goal".to_string()),
            relationship_id,
        );
        let relationship = create_test_relationship(relationship_id);

        // Mock sequence: goal save → relationship lookup (no limit check)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![new_goal.clone()]])
            .append_query_results(vec![vec![relationship]])
            .into_connection();

        let result = create(&db, &event_publisher, new_goal, Id::new_v4()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn update_status_rejects_in_progress_when_at_limit() {
        let relationship_id = Id::new_v4();
        let event_publisher = EventPublisher::new();

        let current_goal = create_test_goal_with(
            Status::NotStarted,
            Some("My goal".to_string()),
            relationship_id,
        );

        let active_goals: Vec<Model> = (0..MAX_ACTIVE_GOALS)
            .map(|i| {
                create_test_goal_with(
                    Status::InProgress,
                    Some(format!("Active {i}")),
                    relationship_id,
                )
            })
            .collect();

        // Mock sequence: find_by_id (current goal) → active goals query → error
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![current_goal.clone()]])
            .append_query_results(vec![active_goals])
            .into_connection();

        let result =
            update_status(&db, &event_publisher, current_goal.id, Status::InProgress).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::ActiveGoalLimitReached { .. })
        ));
    }

    #[tokio::test]
    async fn update_status_allows_in_progress_to_in_progress() {
        let relationship_id = Id::new_v4();
        let event_publisher = EventPublisher::new();

        // Goal is already InProgress — no-op transition, skips limit check
        let current_goal = create_test_goal_with(
            Status::InProgress,
            Some("Already active".to_string()),
            relationship_id,
        );
        let relationship = create_test_relationship(relationship_id);

        // Mock sequence: find_by_id → already InProgress so skip limit check →
        //   update_status (find + save) → relationship lookup
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![current_goal.clone()]])
            .append_query_results(vec![vec![current_goal.clone()]])
            .append_query_results(vec![vec![current_goal.clone()]])
            .append_query_results(vec![vec![relationship]])
            .into_connection();

        let result =
            update_status(&db, &event_publisher, current_goal.id, Status::InProgress).await;

        assert!(result.is_ok());
    }
}
