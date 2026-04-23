//! Data gathering for goal progress computation.
//!
//! This module spans multiple entity types (goals, actions, coaching_sessions_goals)
//! and gathers the raw data needed by the domain layer to compute progress heuristics.

use std::collections::HashMap;

use sea_orm::{
    entity::prelude::*, ConnectionTrait, FromQueryResult, JoinType, Order, QueryOrder, QuerySelect,
    QueryTrait,
};

use log::*;

use super::error::{EntityApiErrorKind, Error};
use entity::{
    actions, actions_users, coaching_sessions, coaching_sessions_goals, goals, status::Status, Id,
};

/// Optional filter/sort/limit parameters for `gather_batch_progress_data`.
///
/// All fields are optional — the default is equivalent to today's behavior
/// (every goal in the relationship, unsorted, unbounded, no assignee scope).
#[derive(Debug, Clone, Default)]
pub struct BatchProgressParams {
    /// Only include goals with this status.
    pub status: Option<Status>,
    /// Sort goals by this column. Paired with `sort_order`; both must be
    /// `Some` for ordering to be applied.
    pub sort_column: Option<goals::Column>,
    /// Sort direction. Paired with `sort_column`.
    pub sort_order: Option<Order>,
    /// Cap on the number of goals returned. Applied as SQL `LIMIT` on the
    /// initial goals query, so stats queries only run for the limited set.
    pub limit: Option<u64>,
    /// When set, restrict `actions_completed` / `actions_total` /
    /// `next_action_due` / `completed_action_dates` to actions assigned to
    /// this user. Session-scoped metrics (count, last date) are unaffected.
    pub assignee_user_id: Option<Id>,
    /// When set, restrict the goals returned to those linked to this
    /// coaching session via the `coaching_sessions_goals` join table.
    pub coaching_session_id: Option<Id>,
}

/// Raw data gathered from multiple entities for progress computation.
pub struct ProgressData {
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

/// Gathers all data needed to compute progress metrics for a goal.
///
/// Queries across goals, actions, and coaching_sessions_goals to build
/// a complete picture of goal progress data.
///
/// # Errors
///
/// Returns `Error` if the goal is not found or any database query fails.
pub async fn gather_progress_data(
    db: &impl ConnectionTrait,
    goal_id: Id,
) -> Result<ProgressData, Error> {
    let goal = find_goal(db, goal_id).await?;
    let actions = find_actions_for_goal(db, goal_id).await?;
    let action_stats = summarize_action_stats(&actions);
    let coaching_session_stats = find_linked_coaching_session_stats(db, goal_id).await?;

    debug!(
        "Progress data for goal {goal_id}: {}/{} actions completed, {} sessions linked",
        action_stats.completed, action_stats.total, coaching_session_stats.count
    );

    Ok(ProgressData {
        goal,
        actions_total: action_stats.total,
        actions_completed: action_stats.completed,
        completed_action_dates: action_stats.completed_dates,
        next_action_due: action_stats.next_due,
        linked_coaching_session_count: coaching_session_stats.count,
        last_coaching_session_date: coaching_session_stats.last_date,
    })
}

/// Aggregate row for action stats per goal.
#[derive(Debug, FromQueryResult)]
struct ActionStatsRow {
    goal_id: Id,
    actions_total: i64,
    actions_completed: i64,
    next_action_due: Option<DateTimeWithTimeZone>,
}

/// Aggregate row for session stats per goal.
#[derive(Debug, FromQueryResult)]
struct SessionStatsRow {
    goal_id: Id,
    linked_coaching_session_count: i64,
    last_coaching_session_date: Option<DateTime>,
}

/// Row for completed action timestamps (momentum-based progress computation).
#[derive(Debug, FromQueryResult)]
struct CompletedDateRow {
    goal_id: Id,
    status_changed_at: DateTimeWithTimeZone,
}

/// Gathers progress data for all goals in a coaching relationship using aggregate queries.
///
/// Uses 3-4 optimized queries regardless of goal count:
/// 1. All goals for the relationship
/// 2. Action stats per goal (total, completed, next due) via `GROUP BY` with `CASE WHEN`
/// 3. Session stats per goal (count, last date) via `GROUP BY` with `JOIN`
/// 4. (conditional) Completed action dates for momentum-based goals only
///
/// # Errors
///
/// Returns `Error` if any database query fails.
pub async fn gather_batch_progress_data(
    db: &impl ConnectionTrait,
    coaching_relationship_id: Id,
    params: BatchProgressParams,
) -> Result<Vec<ProgressData>, Error> {
    // Query 1: Goals for the coaching relationship, with optional filter/sort/limit.
    let query = goals::Entity::find()
        .filter(goals::Column::CoachingRelationshipId.eq(coaching_relationship_id));

    let query = match &params.status {
        Some(status) => query.filter(goals::Column::Status.eq(status.clone())),
        None => query,
    };

    // Restrict to goals linked to a specific coaching session via the
    // `coaching_sessions_goals` join table.
    let query = match params.coaching_session_id {
        Some(session_id) => {
            let session_goals_subquery = coaching_sessions_goals::Entity::find()
                .select_only()
                .column(coaching_sessions_goals::Column::GoalId)
                .filter(coaching_sessions_goals::Column::CoachingSessionId.eq(session_id))
                .into_query();
            query.filter(goals::Column::Id.in_subquery(session_goals_subquery))
        }
        None => query,
    };

    let query = match params.sort_column.zip(params.sort_order) {
        Some((col, ord)) => query.order_by(col, ord),
        None => query,
    };

    let query = match params.limit {
        Some(n) => query.limit(n),
        None => query,
    };

    let goals = query.all(db).await?;

    if goals.is_empty() {
        return Ok(Vec::new());
    }

    let goal_ids: Vec<Id> = goals.iter().map(|g| g.id).collect();

    // Query 2: Action stats aggregated per goal — single query with CASE WHEN
    // for conditional count (completed) and conditional MIN (next due for non-completed).
    // When `assignee_user_id` is set, INNER JOIN actions_users so counts only include
    // actions assigned to that user.
    let action_stats_query = actions::Entity::find()
        .select_only()
        .column(actions::Column::GoalId)
        .column_as(actions::Column::Id.count(), "actions_total")
        .column_as(
            Expr::cust("SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END)"),
            "actions_completed",
        )
        .column_as(
            Expr::cust("MIN(CASE WHEN status != 'completed' THEN due_by END)"),
            "next_action_due",
        )
        .filter(actions::Column::GoalId.is_in(goal_ids.clone()));

    let action_stats_query = match params.assignee_user_id {
        Some(user_id) => action_stats_query
            .join(JoinType::InnerJoin, actions::Relation::ActionsUsers.def())
            .filter(actions_users::Column::UserId.eq(user_id)),
        None => action_stats_query,
    };

    let action_stats_rows: Vec<ActionStatsRow> = action_stats_query
        .group_by(actions::Column::GoalId)
        .into_model::<ActionStatsRow>()
        .all(db)
        .await?;

    let action_stats: HashMap<Id, &ActionStatsRow> =
        action_stats_rows.iter().map(|r| (r.goal_id, r)).collect();

    // Query 3: Session stats aggregated per goal via JOIN to coaching_sessions
    let session_stats_rows: Vec<SessionStatsRow> = coaching_sessions_goals::Entity::find()
        .select_only()
        .column(coaching_sessions_goals::Column::GoalId)
        .column_as(
            coaching_sessions_goals::Column::Id.count(),
            "linked_coaching_session_count",
        )
        .column_as(
            coaching_sessions::Column::Date.max(),
            "last_coaching_session_date",
        )
        .join(
            JoinType::InnerJoin,
            coaching_sessions_goals::Relation::CoachingSessions.def(),
        )
        .filter(coaching_sessions_goals::Column::GoalId.is_in(goal_ids.clone()))
        .group_by(coaching_sessions_goals::Column::GoalId)
        .into_model::<SessionStatsRow>()
        .all(db)
        .await?;

    let session_stats: HashMap<Id, &SessionStatsRow> =
        session_stats_rows.iter().map(|r| (r.goal_id, r)).collect();

    // Query 4 (conditional): Completed action dates for momentum-based goals only.
    // Duration-based goals (with target_date) don't use cadence calculations.
    let momentum_goal_ids: Vec<Id> = goals
        .iter()
        .filter(|g| g.target_date.is_none())
        .map(|g| g.id)
        .collect();

    let completed_dates_by_goal: HashMap<Id, Vec<DateTimeWithTimeZone>> =
        if momentum_goal_ids.is_empty() {
            HashMap::new()
        } else {
            let date_query = actions::Entity::find()
                .select_only()
                .column(actions::Column::GoalId)
                .column(actions::Column::StatusChangedAt)
                .filter(actions::Column::GoalId.is_in(momentum_goal_ids))
                .filter(actions::Column::Status.eq("completed"));

            let date_query = match params.assignee_user_id {
                Some(user_id) => date_query
                    .join(JoinType::InnerJoin, actions::Relation::ActionsUsers.def())
                    .filter(actions_users::Column::UserId.eq(user_id)),
                None => date_query,
            };

            let date_rows: Vec<CompletedDateRow> = date_query
                .order_by_asc(actions::Column::GoalId)
                .order_by_asc(actions::Column::StatusChangedAt)
                .into_model::<CompletedDateRow>()
                .all(db)
                .await?;

            let mut map: HashMap<Id, Vec<DateTimeWithTimeZone>> = HashMap::new();
            for row in date_rows {
                map.entry(row.goal_id)
                    .or_default()
                    .push(row.status_changed_at);
            }
            map
        };

    // Assemble ProgressData for each goal from aggregate results
    let result: Vec<ProgressData> = goals
        .into_iter()
        .map(|goal| {
            let goal_id = goal.id;

            let (actions_total, actions_completed, next_action_due) =
                match action_stats.get(&goal_id) {
                    Some(stats) => (
                        stats.actions_total as usize,
                        stats.actions_completed as usize,
                        stats.next_action_due,
                    ),
                    None => (0, 0, None),
                };

            let (linked_coaching_session_count, last_coaching_session_date) =
                match session_stats.get(&goal_id) {
                    Some(stats) => (
                        stats.linked_coaching_session_count as usize,
                        stats.last_coaching_session_date,
                    ),
                    None => (0, None),
                };

            let completed_action_dates = completed_dates_by_goal
                .get(&goal_id)
                .cloned()
                .unwrap_or_default();

            ProgressData {
                goal,
                actions_total,
                actions_completed,
                completed_action_dates,
                next_action_due,
                linked_coaching_session_count,
                last_coaching_session_date,
            }
        })
        .collect();

    debug!(
        "Batch progress data for relationship {coaching_relationship_id}: {} goals",
        result.len()
    );

    Ok(result)
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
        create_test_goal_for_relationship(Id::new_v4(), target_date)
    }

    fn create_test_goal_for_relationship(
        coaching_relationship_id: Id,
        target_date: Option<Date>,
    ) -> goals::Model {
        let now = chrono::Utc::now().fixed_offset();
        goals::Model {
            id: Id::new_v4(),
            coaching_relationship_id,
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
    async fn gather_progress_data_returns_data_for_goal_with_no_actions_or_sessions(
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

        let data = gather_progress_data(&db, goal_id).await?;

        assert_eq!(data.goal.id, goal_id);
        assert_eq!(data.actions_total, 0);
        assert_eq!(data.actions_completed, 0);
        assert_eq!(data.linked_coaching_session_count, 0);
        assert!(data.last_coaching_session_date.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn gather_batch_progress_data_returns_empty_for_no_goals() -> Result<(), Error> {
        let relationship_id = Id::new_v4();

        // Query 1: goals → empty (no further queries executed)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let result =
            gather_batch_progress_data(&db, relationship_id, BatchProgressParams::default())
                .await?;

        assert!(result.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn gather_batch_progress_data_assembles_data_for_duration_based_goals(
    ) -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let target = chrono::Utc::now().date_naive() + chrono::Duration::days(30);
        let goal1 = create_test_goal_for_relationship(relationship_id, Some(target));
        let goal2 = create_test_goal_for_relationship(relationship_id, Some(target));

        // All goals have target_date → momentum_goal_ids is empty → query 4 is skipped.
        // Mock sequence: goals → action stats (empty) → session stats (empty)
        // Empty result sets use goals::Model as a dummy type (type is irrelevant for 0 rows).
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal1.clone(), goal2.clone()]])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let result =
            gather_batch_progress_data(&db, relationship_id, BatchProgressParams::default())
                .await?;

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].goal.id, goal1.id);
        assert_eq!(result[1].goal.id, goal2.id);
        // No actions or sessions → defaults to zero
        for data in &result {
            assert_eq!(data.actions_total, 0);
            assert_eq!(data.actions_completed, 0);
            assert_eq!(data.linked_coaching_session_count, 0);
            assert!(data.next_action_due.is_none());
            assert!(data.last_coaching_session_date.is_none());
            assert!(data.completed_action_dates.is_empty());
        }

        Ok(())
    }

    #[tokio::test]
    async fn gather_batch_progress_data_runs_completed_dates_query_for_momentum_goals(
    ) -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        // One momentum-based goal (no target_date) → query 4 executes
        let goal = create_test_goal_for_relationship(relationship_id, None);

        // Mock sequence: goals → action stats (empty) → session stats (empty) → completed dates (empty)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal.clone()]])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let result =
            gather_batch_progress_data(&db, relationship_id, BatchProgressParams::default())
                .await?;

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].goal.id, goal.id);
        assert!(result[0].goal.target_date.is_none());
        assert!(result[0].completed_action_dates.is_empty());

        // Verify that 4 queries were executed (not 3)
        let log = db.into_transaction_log();
        assert_eq!(log.len(), 4);

        Ok(())
    }

    // ── Push-down SQL tests ────────────────────────────────────────────
    //
    // These tests prove that filter/sort/limit/assignee parameters are
    // applied as SQL predicates rather than post-query filters. They use
    // MockDatabase::into_transaction_log() to inspect the generated SQL,
    // following the same pattern as
    // `action::tests::find_by_user_with_sessions_scope_and_relationship_filter`.

    #[tokio::test]
    async fn status_filter_pushed_to_goals_sql() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        // Empty goals → short-circuits after Query 1; only one transaction to inspect.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let _ = gather_batch_progress_data(
            &db,
            relationship_id,
            BatchProgressParams {
                status: Some(Status::InProgress),
                ..Default::default()
            },
        )
        .await?;

        let log = db.into_transaction_log();
        let sql = format!("{:?}", log[0]);
        assert!(
            sql.contains(r#"\"goals\".\"status\" ="#),
            "Query 1 should filter by goals.status, got: {sql}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn coaching_session_id_emits_in_subquery() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let session_id = Id::new_v4();
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let _ = gather_batch_progress_data(
            &db,
            relationship_id,
            BatchProgressParams {
                coaching_session_id: Some(session_id),
                ..Default::default()
            },
        )
        .await?;

        let log = db.into_transaction_log();
        let sql = format!("{:?}", log[0]);
        // Expect: goals.id IN (SELECT goal_id FROM coaching_sessions_goals WHERE coaching_session_id = $N)
        assert!(
            sql.contains(r#"\"goals\".\"id\" IN (SELECT"#),
            "Query 1 should use a subquery to filter goals by coaching_session_id, got: {sql}"
        );
        assert!(
            sql.contains(r#"\"coaching_sessions_goals\""#)
                && sql.contains(r#"\"coaching_session_id\" ="#),
            "subquery should select from coaching_sessions_goals filtered by coaching_session_id, got: {sql}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn limit_emits_limit_clause() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let _ = gather_batch_progress_data(
            &db,
            relationship_id,
            BatchProgressParams {
                limit: Some(3),
                ..Default::default()
            },
        )
        .await?;

        let log = db.into_transaction_log();
        let sql = format!("{:?}", log[0]);
        assert!(
            sql.contains("LIMIT"),
            "Query 1 should emit a LIMIT clause when params.limit is set, got: {sql}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn assignee_user_id_emits_actions_users_join_in_stats() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let assignee_user_id = Id::new_v4();
        // Duration-based goal (target_date is Some) → Query 4 is skipped, simpler mock.
        let target = chrono::Utc::now().date_naive() + chrono::Duration::days(30);
        let goal = create_test_goal_for_relationship(relationship_id, Some(target));

        // Mock sequence: goals → action stats (empty) → session stats (empty)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal.clone()]])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let _ = gather_batch_progress_data(
            &db,
            relationship_id,
            BatchProgressParams {
                assignee_user_id: Some(assignee_user_id),
                ..Default::default()
            },
        )
        .await?;

        let log = db.into_transaction_log();
        // Query 2 is the action-stats aggregate — where the actions_users join lives.
        // Tables are schema-qualified (`"refactor_platform"."actions_users"`), so we
        // match on the keyword + table name rather than an exact phrase.
        let query2_sql = format!("{:?}", log[1]);
        assert!(
            query2_sql.contains("INNER JOIN") && query2_sql.contains(r#"\"actions_users\""#),
            "Query 2 should INNER JOIN actions_users when assignee_user_id is set, got: {query2_sql}"
        );
        assert!(
            query2_sql.contains(r#"\"actions_users\".\"user_id\" ="#),
            "Query 2 should filter actions_users.user_id, got: {query2_sql}"
        );

        Ok(())
    }
}
