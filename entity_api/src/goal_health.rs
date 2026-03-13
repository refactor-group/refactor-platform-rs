//! Data gathering for goal health computation.
//!
//! This module spans multiple entity types (goals, actions, coaching_sessions_goals)
//! and gathers the raw data needed by the domain layer to compute health heuristics.

use sea_orm::{entity::prelude::*, ConnectionTrait};

use log::*;

use super::error::{EntityApiErrorKind, Error};
use entity::{actions, coaching_sessions, coaching_sessions_goals, goals, status::Status, Id};

/// Raw data gathered from multiple entities for health computation.
pub struct HealthData {
    /// The goal itself.
    pub goal: goals::Model,
    /// Total number of actions linked to this goal.
    pub actions_total: usize,
    /// Number of completed actions.
    pub actions_completed: usize,
    /// Timestamps when each action was completed (`status_changed_at` for completed actions),
    /// sorted chronologically.
    pub completed_action_dates: Vec<DateTimeWithTimeZone>,
    /// Earliest `due_by` date among non-completed actions.
    pub next_action_due: Option<DateTimeWithTimeZone>,
    /// Number of coaching sessions linked to this goal.
    pub linked_coaching_session_count: usize,
    /// Date of the most recent linked coaching session.
    pub last_coaching_session_date: Option<DateTime>,
}

/// Gathers all data needed to compute health metrics for a goal.
///
/// Queries across goals, actions, and coaching_sessions_goals to build
/// a complete picture of goal health data.
///
/// # Errors
///
/// Returns `Error` if the goal is not found or any database query fails.
pub async fn gather_health_data(
    db: &impl ConnectionTrait,
    goal_id: Id,
) -> Result<HealthData, Error> {
    let goal = find_goal(db, goal_id).await?;
    let actions = find_actions_for_goal(db, goal_id).await?;
    let action_stats = summarize_action_stats(&actions);
    let coaching_session_stats = find_linked_coaching_session_stats(db, goal_id).await?;

    debug!(
        "Health data for goal {goal_id}: {}/{} actions completed, {} sessions linked",
        action_stats.completed, action_stats.total, coaching_session_stats.count
    );

    Ok(HealthData {
        goal,
        actions_total: action_stats.total,
        actions_completed: action_stats.completed,
        completed_action_dates: action_stats.completed_dates,
        next_action_due: action_stats.next_due,
        linked_coaching_session_count: coaching_session_stats.count,
        last_coaching_session_date: coaching_session_stats.last_date,
    })
}

// ── Private helpers ────────────────────────────────────────────────────

async fn find_goal(db: &impl ConnectionTrait, goal_id: Id) -> Result<goals::Model, Error> {
    goals::Entity::find_by_id(goal_id)
        .one(db)
        .await?
        .ok_or(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })
}

async fn find_actions_for_goal(
    db: &impl ConnectionTrait,
    goal_id: Id,
) -> Result<Vec<actions::Model>, Error> {
    Ok(actions::Entity::find()
        .filter(actions::Column::GoalId.eq(goal_id))
        .all(db)
        .await?)
}

struct ActionStats {
    total: usize,
    completed: usize,
    completed_dates: Vec<DateTimeWithTimeZone>,
    next_due: Option<DateTimeWithTimeZone>,
}

fn summarize_action_stats(actions: &[actions::Model]) -> ActionStats {
    let total = actions.len();
    let mut completed = 0;
    let mut completed_dates = Vec::new();
    let mut next_due: Option<DateTimeWithTimeZone> = None;

    for action in actions {
        if action.status == Status::Completed {
            completed += 1;
            // Note: status_changed_at reflects creation time if the action was created
            // directly as Completed (rare). This is acceptable for cadence heuristics.
            completed_dates.push(action.status_changed_at);
        } else if let Some(due) = action.due_by {
            next_due = Some(match next_due {
                Some(current) if due < current => due,
                Some(current) => current,
                None => due,
            });
        }
    }

    // Sort chronologically for cadence calculations in the domain layer
    completed_dates.sort();

    ActionStats {
        total,
        completed,
        completed_dates,
        next_due,
    }
}

struct CoachingSessionStats {
    count: usize,
    last_date: Option<DateTime>,
}

async fn find_linked_coaching_session_stats(
    db: &impl ConnectionTrait,
    goal_id: Id,
) -> Result<CoachingSessionStats, Error> {
    let links_with_sessions = coaching_sessions_goals::Entity::find()
        .filter(coaching_sessions_goals::Column::GoalId.eq(goal_id))
        .find_also_related(coaching_sessions::Entity)
        .all(db)
        .await?;

    let mut count = 0;
    let mut last_date: Option<DateTime> = None;

    for (_, session) in &links_with_sessions {
        if let Some(session) = session {
            count += 1;
            last_date = Some(match last_date {
                Some(current) if session.date > current => session.date,
                Some(current) => current,
                None => session.date,
            });
        }
    }

    Ok(CoachingSessionStats { count, last_date })
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::status::Status;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn create_test_goal(target_date: Option<Date>) -> goals::Model {
        let now = chrono::Utc::now().fixed_offset();
        goals::Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            created_in_session_id: None,
            user_id: Id::new_v4(),
            title: Some("Test goal".to_string()),
            body: None,
            status: Status::InProgress,
            status_changed_at: None,
            completed_at: None,
            target_date,
            created_at: now,
            updated_at: now,
        }
    }

    fn create_test_action(
        goal_id: Id,
        status: Status,
        due_by: Option<DateTimeWithTimeZone>,
    ) -> actions::Model {
        let now = chrono::Utc::now().fixed_offset();
        actions::Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            goal_id: Some(goal_id),
            user_id: Id::new_v4(),
            body: Some("Test action".to_string()),
            due_by,
            status,
            status_changed_at: now,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn summarize_action_stats_counts_correctly() {
        let goal_id = Id::new_v4();
        let now = chrono::Utc::now().fixed_offset();
        let tomorrow = now + chrono::Duration::days(1);

        let actions = vec![
            create_test_action(goal_id, Status::Completed, None),
            create_test_action(goal_id, Status::Completed, None),
            create_test_action(goal_id, Status::InProgress, Some(tomorrow)),
            create_test_action(goal_id, Status::NotStarted, Some(now)),
        ];

        let stats = summarize_action_stats(&actions);

        assert_eq!(stats.total, 4);
        assert_eq!(stats.completed, 2);
        assert_eq!(stats.completed_dates.len(), 2);
        // next_due should be the earliest due_by among non-completed actions
        assert_eq!(stats.next_due, Some(now));
    }

    #[test]
    fn summarize_action_stats_handles_empty() {
        let stats = summarize_action_stats(&[]);

        assert_eq!(stats.total, 0);
        assert_eq!(stats.completed, 0);
        assert!(stats.completed_dates.is_empty());
        assert!(stats.next_due.is_none());
    }

    #[tokio::test]
    async fn gather_health_data_returns_data_for_goal_with_no_actions_or_sessions(
    ) -> Result<(), Error> {
        let goal = create_test_goal(None);
        let goal_id = goal.id;

        // Mock sequence: find_goal → find_actions (empty) → find_linked_sessions (empty)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal.clone()]])
            .append_query_results(vec![Vec::<actions::Model>::new()])
            .append_query_results(vec![Vec::<(
                coaching_sessions_goals::Model,
                Option<coaching_sessions::Model>,
            )>::new()])
            .into_connection();

        let data = gather_health_data(&db, goal_id).await?;

        assert_eq!(data.goal.id, goal_id);
        assert_eq!(data.actions_total, 0);
        assert_eq!(data.actions_completed, 0);
        assert_eq!(data.linked_coaching_session_count, 0);
        assert!(data.last_coaching_session_date.is_none());

        Ok(())
    }
}
