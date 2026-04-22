//! Goal progress computation.
//!
//! Computes progress heuristics for a goal based on action completion,
//! coaching session engagement, and optional target date timelines.
//!
//! Two modes of computation:
//! - **Duration-based**: When `target_date` is set, compares elapsed time against action progress.
//! - **Momentum-based**: When `target_date` is null, looks at action completion cadence.

use chrono::{NaiveDate, Utc};
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::ConnectionTrait;
use serde::Serialize;

use crate::error::Error;
use crate::status::Status;
use crate::Id;

pub use entity_api::goal_progress::BatchProgressParams;

/// Overall progress signal for a goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Progress {
    SolidMomentum,
    NeedsAttention,
    LetsRefocus,
}

/// Computed progress metrics returned by the progress endpoint.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProgressMetrics {
    pub progress: Progress,
    pub actions_completed: usize,
    pub actions_total: usize,
    pub linked_coaching_session_count: usize,
    pub last_coaching_session_date: Option<chrono::NaiveDateTime>,
    pub next_action_due: Option<DateTimeWithTimeZone>,
}

/// Computes progress metrics for a goal by gathering data and applying heuristics.
///
/// # Errors
///
/// Returns `Error` if the goal is not found or any database query fails.
pub async fn progress_metrics(
    db: &impl ConnectionTrait,
    goal_id: Id,
) -> Result<ProgressMetrics, Error> {
    let data = entity_api::goal_progress::gather_progress_data(db, goal_id).await?;
    let progress = compute_progress(&data);

    Ok(ProgressMetrics {
        progress,
        actions_completed: data.actions_completed,
        actions_total: data.actions_total,
        linked_coaching_session_count: data.linked_coaching_session_count,
        last_coaching_session_date: data.last_coaching_session_date,
        next_action_due: data.next_action_due,
    })
}

/// A single goal's identity combined with its computed progress metrics.
#[derive(Debug, Clone, Serialize)]
pub struct GoalProgressEntry {
    pub goal_id: Id,
    pub coaching_relationship_id: Id,
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Status,
    pub status_changed_at: Option<DateTimeWithTimeZone>,
    pub target_date: Option<NaiveDate>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    pub progress_metrics: ProgressMetrics,
}

/// Aggregate goal progress for all goals in a coaching relationship.
#[derive(Debug, Clone, Serialize)]
pub struct RelationshipGoalProgress {
    pub goal_progress: Vec<GoalProgressEntry>,
}

/// Computes progress metrics for all goals in a coaching relationship.
///
/// Uses batch-optimized aggregate queries (3-4 total, regardless of goal count)
/// to gather data, then applies the same `compute_progress()` heuristics used
/// by the single-goal endpoint.
///
/// # Errors
///
/// Returns `Error` if any database query fails.
pub async fn relationship_goal_progress(
    db: &impl ConnectionTrait,
    coaching_relationship_id: Id,
    params: BatchProgressParams,
) -> Result<RelationshipGoalProgress, Error> {
    let batch_data =
        entity_api::goal_progress::gather_batch_progress_data(db, coaching_relationship_id, params)
            .await?;

    let goal_progress = batch_data
        .into_iter()
        .map(|data| {
            let progress = compute_progress(&data);

            let metrics = ProgressMetrics {
                progress,
                actions_completed: data.actions_completed,
                actions_total: data.actions_total,
                linked_coaching_session_count: data.linked_coaching_session_count,
                last_coaching_session_date: data.last_coaching_session_date,
                next_action_due: data.next_action_due,
            };

            GoalProgressEntry {
                goal_id: data.goal.id,
                coaching_relationship_id: data.goal.coaching_relationship_id,
                title: data.goal.title,
                body: data.goal.body,
                status: data.goal.status,
                status_changed_at: data.goal.status_changed_at,
                target_date: data.goal.target_date,
                created_at: data.goal.created_at,
                updated_at: data.goal.updated_at,
                progress_metrics: metrics,
            }
        })
        .collect();

    Ok(RelationshipGoalProgress { goal_progress })
}

// ── Progress heuristics ───────────────────────────────────────────────

/// Grace period for brand-new goals (days). Goals younger than this
/// default to `SolidMomentum` regardless of other signals.
const NEW_GOAL_GRACE_PERIOD_DAYS: i64 = 7;

fn compute_progress(data: &entity_api::goal_progress::ProgressData) -> Progress {
    let now = Utc::now().fixed_offset();
    let created_at = data.goal.created_at;

    // Brand-new goals get a grace period
    let age_days = (now - created_at).num_days();
    if age_days < NEW_GOAL_GRACE_PERIOD_DAYS {
        return Progress::SolidMomentum;
    }

    match data.goal.target_date {
        Some(target_date) => compute_duration_based_progress(data, target_date),
        None => compute_momentum_based_progress(data),
    }
}

/// Duration-based progress: compares elapsed time against action completion.
///
/// - No actions defined → `NeedsAttention` (goal not broken down yet)
/// - Overdue → `LetsRefocus`
/// - `progress_pct < elapsed_pct * 0.5` → `LetsRefocus` (significantly behind)
/// - `progress_pct < elapsed_pct` → `NeedsAttention` (falling behind)
/// - `progress_pct >= elapsed_pct` → `SolidMomentum` (on track or ahead)
fn compute_duration_based_progress(
    data: &entity_api::goal_progress::ProgressData,
    target_date: NaiveDate,
) -> Progress {
    // No actions defined yet — can't measure progress against timeline
    if data.actions_total == 0 {
        return Progress::NeedsAttention;
    }

    let now = Utc::now().date_naive();
    let created_date = data.goal.created_at.date_naive();

    // Overdue goals always get LetsRefocus
    if now > target_date {
        return Progress::LetsRefocus;
    }

    let total_duration = (target_date - created_date).num_days() as f64;
    if total_duration <= 0.0 {
        return Progress::NeedsAttention;
    }

    let elapsed = (now - created_date).num_days() as f64;
    let elapsed_pct = elapsed / total_duration;

    let progress_pct = data.actions_completed as f64 / data.actions_total as f64;

    if progress_pct < elapsed_pct * 0.5 {
        Progress::LetsRefocus
    } else if progress_pct < elapsed_pct {
        Progress::NeedsAttention
    } else {
        Progress::SolidMomentum
    }
}

/// Momentum-based progress: looks at action completion cadence when no target date is set.
///
/// - Zero actions → `NeedsAttention` (goal not broken down yet)
/// - Zero completed → `NeedsAttention` (no progress)
/// - Last completion within 2× expected cadence → `SolidMomentum`
/// - Last completion within 4× expected cadence → `NeedsAttention`
/// - Beyond 4× → `LetsRefocus`
fn compute_momentum_based_progress(data: &entity_api::goal_progress::ProgressData) -> Progress {
    if data.actions_total == 0 {
        return Progress::NeedsAttention;
    }

    if data.actions_completed == 0 {
        return Progress::NeedsAttention;
    }

    // Expected cadence: average days between completions based on goal lifetime
    let now = Utc::now().fixed_offset();
    let age_days = (now - data.goal.created_at).num_days().max(1) as f64;
    let expected_cadence_days = age_days / data.actions_completed as f64;

    // Days since the most recent completion
    let days_since_last_completion = data
        .completed_action_dates
        .last()
        .map(|d| (now - *d).num_days() as f64)
        .unwrap_or(age_days);

    if days_since_last_completion <= expected_cadence_days * 2.0 {
        Progress::SolidMomentum
    } else if days_since_last_completion <= expected_cadence_days * 4.0 {
        Progress::NeedsAttention
    } else {
        Progress::LetsRefocus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use entity_api::goal_progress::ProgressData;

    use crate::goals;
    use crate::status::Status;

    fn create_test_progress_data(
        target_date: Option<NaiveDate>,
        age_days: i64,
        actions_total: usize,
        actions_completed: usize,
    ) -> ProgressData {
        let now = Utc::now().fixed_offset();
        let created_at = now - Duration::days(age_days);

        ProgressData {
            goal: goals::Model {
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
                created_at,
                updated_at: now,
            },
            actions_total,
            actions_completed,
            completed_action_dates: Vec::new(),
            next_action_due: None,
            linked_coaching_session_count: 0,
            last_coaching_session_date: None,
        }
    }

    #[test]
    fn brand_new_goal_gets_solid_momentum() {
        let data = create_test_progress_data(None, 3, 0, 0);
        assert_eq!(compute_progress(&data), Progress::SolidMomentum);
    }

    #[test]
    fn overdue_goal_gets_lets_refocus() {
        let yesterday = Utc::now().date_naive() - Duration::days(1);
        let data = create_test_progress_data(Some(yesterday), 30, 5, 1);
        assert_eq!(compute_progress(&data), Progress::LetsRefocus);
    }

    #[test]
    fn on_track_goal_gets_solid_momentum() {
        // 50% elapsed, 60% completed → on track
        let target = Utc::now().date_naive() + Duration::days(30);
        let data = create_test_progress_data(Some(target), 30, 10, 6);
        assert_eq!(compute_progress(&data), Progress::SolidMomentum);
    }

    #[test]
    fn falling_behind_goal_gets_needs_attention() {
        // 50% elapsed, 40% completed → behind but not significantly
        let target = Utc::now().date_naive() + Duration::days(30);
        let data = create_test_progress_data(Some(target), 30, 10, 4);
        assert_eq!(compute_progress(&data), Progress::NeedsAttention);
    }

    #[test]
    fn significantly_behind_goal_gets_lets_refocus() {
        // 50% elapsed, 20% completed → less than half expected progress
        let target = Utc::now().date_naive() + Duration::days(30);
        let data = create_test_progress_data(Some(target), 30, 10, 2);
        assert_eq!(compute_progress(&data), Progress::LetsRefocus);
    }

    #[test]
    fn no_actions_with_target_date_gets_needs_attention() {
        // Goal has a target date but no actions defined yet → not broken down
        let target = Utc::now().date_naive() + Duration::days(30);
        let data = create_test_progress_data(Some(target), 14, 0, 0);
        assert_eq!(compute_progress(&data), Progress::NeedsAttention);
    }

    #[test]
    fn no_actions_with_no_target_date_gets_needs_attention() {
        let data = create_test_progress_data(None, 14, 0, 0);
        assert_eq!(compute_progress(&data), Progress::NeedsAttention);
    }

    #[test]
    fn no_completed_actions_gets_needs_attention() {
        let data = create_test_progress_data(None, 14, 5, 0);
        assert_eq!(compute_progress(&data), Progress::NeedsAttention);
    }

    #[test]
    fn momentum_based_recent_completion_gets_solid_momentum() {
        let now = Utc::now().fixed_offset();
        let mut data = create_test_progress_data(None, 30, 5, 3);
        // 3 completions in 30 days → ~10-day cadence
        // Last completion 5 days ago → within 2× cadence (20 days)
        data.completed_action_dates = vec![
            now - Duration::days(25),
            now - Duration::days(15),
            now - Duration::days(5),
        ];
        assert_eq!(compute_progress(&data), Progress::SolidMomentum);
    }

    #[test]
    fn momentum_based_stale_completion_gets_needs_attention() {
        let now = Utc::now().fixed_offset();
        let mut data = create_test_progress_data(None, 30, 5, 3);
        // 3 completions in 30 days → ~10-day cadence
        // Last completion 25 days ago → within 4× cadence (40 days) but beyond 2× (20 days)
        data.completed_action_dates = vec![
            now - Duration::days(29),
            now - Duration::days(28),
            now - Duration::days(25),
        ];
        assert_eq!(compute_progress(&data), Progress::NeedsAttention);
    }

    #[test]
    fn momentum_based_very_stale_gets_lets_refocus() {
        let now = Utc::now().fixed_offset();
        // 5 completions in 50 days → 10-day cadence
        // All completions happened 45-49 days ago → last was 45 days ago
        // 45 / 10 = 4.5× → LetsRefocus
        let mut data = create_test_progress_data(None, 50, 10, 5);
        data.completed_action_dates = vec![
            now - Duration::days(49),
            now - Duration::days(48),
            now - Duration::days(47),
            now - Duration::days(46),
            now - Duration::days(45),
        ];
        assert_eq!(compute_progress(&data), Progress::LetsRefocus);
    }
}

/// Tests for `relationship_goal_progress` that require MockDatabase.
#[cfg(test)]
#[cfg(feature = "mock")]
mod batch_tests {
    use super::*;
    use chrono::Duration;
    use sea_orm::{DatabaseBackend, MockDatabase};

    use crate::goals;
    use crate::status::Status;

    fn create_test_goal_for_relationship(
        coaching_relationship_id: Id,
        target_date: Option<NaiveDate>,
        age_days: i64,
    ) -> goals::Model {
        let now = Utc::now().fixed_offset();
        let created_at = now - Duration::days(age_days);
        goals::Model {
            id: Id::new_v4(),
            coaching_relationship_id,
            created_in_session_id: None,
            user_id: Id::new_v4(),
            title: Some("Test goal".to_string()),
            body: Some("Test body".to_string()),
            status: Status::InProgress,
            status_changed_at: None,
            completed_at: None,
            target_date,
            created_at,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn relationship_goal_progress_returns_empty_for_no_goals() {
        let relationship_id = Id::new_v4();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let result =
            relationship_goal_progress(&db, relationship_id, BatchProgressParams::default())
                .await
                .unwrap();

        assert!(result.goal_progress.is_empty());
    }

    #[tokio::test]
    async fn relationship_goal_progress_assembles_entries_with_correct_fields() {
        let relationship_id = Id::new_v4();
        let target = Utc::now().date_naive() + Duration::days(30);
        let goal = create_test_goal_for_relationship(relationship_id, Some(target), 14);

        // All goals have target_date → 3 queries (no completed dates query)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal.clone()]])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let result =
            relationship_goal_progress(&db, relationship_id, BatchProgressParams::default())
                .await
                .unwrap();

        assert_eq!(result.goal_progress.len(), 1);

        let entry = &result.goal_progress[0];
        assert_eq!(entry.goal_id, goal.id);
        assert_eq!(entry.coaching_relationship_id, relationship_id);
        assert_eq!(entry.title, Some("Test goal".to_string()));
        assert_eq!(entry.body, Some("Test body".to_string()));
        assert_eq!(entry.status, Status::InProgress);
        assert_eq!(entry.target_date, Some(target));
        assert_eq!(entry.progress_metrics.actions_total, 0);
        assert_eq!(entry.progress_metrics.actions_completed, 0);
        assert_eq!(entry.progress_metrics.linked_coaching_session_count, 0);
    }

    #[tokio::test]
    async fn relationship_goal_progress_computes_progress_per_goal() {
        let relationship_id = Id::new_v4();
        // Brand new goal (3 days old) → SolidMomentum (grace period)
        let new_goal = create_test_goal_for_relationship(
            relationship_id,
            Some(Utc::now().date_naive() + Duration::days(30)),
            3,
        );
        // Old goal with no actions (14 days old, no target) → NeedsAttention
        let old_goal = create_test_goal_for_relationship(relationship_id, None, 14);

        // 4 queries: goals, action stats, session stats, completed dates (for momentum goal)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![new_goal.clone(), old_goal.clone()]])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let result =
            relationship_goal_progress(&db, relationship_id, BatchProgressParams::default())
                .await
                .unwrap();

        assert_eq!(result.goal_progress.len(), 2);
        // Brand new goal gets grace period → SolidMomentum
        assert_eq!(
            result.goal_progress[0].progress_metrics.progress,
            Progress::SolidMomentum
        );
        // Old momentum goal with 0 actions → NeedsAttention
        assert_eq!(
            result.goal_progress[1].progress_metrics.progress,
            Progress::NeedsAttention
        );
    }
}
