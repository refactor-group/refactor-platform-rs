//! Goal health computation.
//!
//! Computes health heuristics for a goal based on action progress,
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
use crate::Id;

/// Overall health signal for a goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    SolidMomentum,
    NeedsAttention,
    LetsRefocus,
}

/// Computed health metrics returned by the health endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct HealthMetrics {
    pub health: Health,
    pub actions_completed: usize,
    pub actions_total: usize,
    pub linked_coaching_session_count: usize,
    pub last_coaching_session_date: Option<chrono::NaiveDateTime>,
    pub next_action_due: Option<DateTimeWithTimeZone>,
}

/// Computes health metrics for a goal by gathering data and applying heuristics.
///
/// # Errors
///
/// Returns `Error` if the goal is not found or any database query fails.
pub async fn health_metrics(
    db: &impl ConnectionTrait,
    goal_id: Id,
) -> Result<HealthMetrics, Error> {
    let data = entity_api::goal_health::gather_health_data(db, goal_id).await?;
    let health = compute_health(&data);

    Ok(HealthMetrics {
        health,
        actions_completed: data.actions_completed,
        actions_total: data.actions_total,
        linked_coaching_session_count: data.linked_coaching_session_count,
        last_coaching_session_date: data.last_coaching_session_date,
        next_action_due: data.next_action_due,
    })
}

// ── Health heuristics ──────────────────────────────────────────────────

/// Grace period for brand-new goals (days). Goals younger than this
/// default to `SolidMomentum` regardless of other signals.
const NEW_GOAL_GRACE_PERIOD_DAYS: i64 = 7;

fn compute_health(data: &entity_api::goal_health::HealthData) -> Health {
    let now = Utc::now().fixed_offset();
    let created_at = data.goal.created_at;

    // Brand-new goals get a grace period
    let age_days = (now - created_at).num_days();
    if age_days < NEW_GOAL_GRACE_PERIOD_DAYS {
        return Health::SolidMomentum;
    }

    match data.goal.target_date {
        Some(target_date) => compute_duration_based_health(data, target_date),
        None => compute_momentum_based_health(data),
    }
}

/// Duration-based health: compares elapsed time against action progress.
///
/// - Overdue → `LetsRefocus`
/// - `progress_pct < elapsed_pct * 0.5` → `LetsRefocus` (significantly behind)
/// - `progress_pct < elapsed_pct` → `NeedsAttention` (falling behind)
/// - `progress_pct >= elapsed_pct` → `SolidMomentum` (on track or ahead)
fn compute_duration_based_health(
    data: &entity_api::goal_health::HealthData,
    target_date: NaiveDate,
) -> Health {
    let now = Utc::now().date_naive();
    let created_date = data.goal.created_at.date_naive();

    // Overdue goals always get LetsRefocus
    if now > target_date {
        return Health::LetsRefocus;
    }

    let total_duration = (target_date - created_date).num_days() as f64;
    if total_duration <= 0.0 {
        return Health::NeedsAttention;
    }

    let elapsed = (now - created_date).num_days() as f64;
    let elapsed_pct = elapsed / total_duration;

    let progress_pct = if data.actions_total == 0 {
        // No actions defined yet — treat as no measurable progress
        0.0
    } else {
        data.actions_completed as f64 / data.actions_total as f64
    };

    if progress_pct < elapsed_pct * 0.5 {
        Health::LetsRefocus
    } else if progress_pct < elapsed_pct {
        Health::NeedsAttention
    } else {
        Health::SolidMomentum
    }
}

/// Momentum-based health: looks at action completion cadence when no target date is set.
///
/// - Zero actions → `NeedsAttention` (goal not broken down yet)
/// - Zero completed → `NeedsAttention` (no progress)
/// - Last completion within 2× expected cadence → `SolidMomentum`
/// - Last completion within 4× expected cadence → `NeedsAttention`
/// - Beyond 4× → `LetsRefocus`
fn compute_momentum_based_health(data: &entity_api::goal_health::HealthData) -> Health {
    if data.actions_total == 0 {
        return Health::NeedsAttention;
    }

    if data.actions_completed == 0 {
        return Health::NeedsAttention;
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
        Health::SolidMomentum
    } else if days_since_last_completion <= expected_cadence_days * 4.0 {
        Health::NeedsAttention
    } else {
        Health::LetsRefocus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use entity_api::goal_health::HealthData;

    use crate::goals;
    use crate::status::Status;

    fn create_test_health_data(
        target_date: Option<NaiveDate>,
        age_days: i64,
        actions_total: usize,
        actions_completed: usize,
    ) -> HealthData {
        let now = Utc::now().fixed_offset();
        let created_at = now - Duration::days(age_days);

        HealthData {
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
        let data = create_test_health_data(None, 3, 0, 0);
        assert_eq!(compute_health(&data), Health::SolidMomentum);
    }

    #[test]
    fn overdue_goal_gets_lets_refocus() {
        let yesterday = Utc::now().date_naive() - Duration::days(1);
        let data = create_test_health_data(Some(yesterday), 30, 5, 1);
        assert_eq!(compute_health(&data), Health::LetsRefocus);
    }

    #[test]
    fn on_track_goal_gets_solid_momentum() {
        // 50% elapsed, 60% completed → on track
        let target = Utc::now().date_naive() + Duration::days(30);
        let data = create_test_health_data(Some(target), 30, 10, 6);
        assert_eq!(compute_health(&data), Health::SolidMomentum);
    }

    #[test]
    fn falling_behind_goal_gets_needs_attention() {
        // 50% elapsed, 40% completed → behind but not significantly
        let target = Utc::now().date_naive() + Duration::days(30);
        let data = create_test_health_data(Some(target), 30, 10, 4);
        assert_eq!(compute_health(&data), Health::NeedsAttention);
    }

    #[test]
    fn significantly_behind_goal_gets_lets_refocus() {
        // 50% elapsed, 20% completed → less than half expected progress
        let target = Utc::now().date_naive() + Duration::days(30);
        let data = create_test_health_data(Some(target), 30, 10, 2);
        assert_eq!(compute_health(&data), Health::LetsRefocus);
    }

    #[test]
    fn no_actions_with_no_target_date_gets_needs_attention() {
        let data = create_test_health_data(None, 14, 0, 0);
        assert_eq!(compute_health(&data), Health::NeedsAttention);
    }

    #[test]
    fn no_completed_actions_gets_needs_attention() {
        let data = create_test_health_data(None, 14, 5, 0);
        assert_eq!(compute_health(&data), Health::NeedsAttention);
    }

    #[test]
    fn momentum_based_recent_completion_gets_solid_momentum() {
        let now = Utc::now().fixed_offset();
        let mut data = create_test_health_data(None, 30, 5, 3);
        // 3 completions in 30 days → ~10-day cadence
        // Last completion 5 days ago → within 2× cadence (20 days)
        data.completed_action_dates = vec![
            now - Duration::days(25),
            now - Duration::days(15),
            now - Duration::days(5),
        ];
        assert_eq!(compute_health(&data), Health::SolidMomentum);
    }

    #[test]
    fn momentum_based_stale_completion_gets_needs_attention() {
        let now = Utc::now().fixed_offset();
        let mut data = create_test_health_data(None, 30, 5, 3);
        // 3 completions in 30 days → ~10-day cadence
        // Last completion 25 days ago → within 4× cadence (40 days) but beyond 2× (20 days)
        data.completed_action_dates = vec![
            now - Duration::days(29),
            now - Duration::days(28),
            now - Duration::days(25),
        ];
        assert_eq!(compute_health(&data), Health::NeedsAttention);
    }

    #[test]
    fn momentum_based_very_stale_gets_lets_refocus() {
        let now = Utc::now().fixed_offset();
        // 5 completions in 50 days → 10-day cadence
        // All completions happened 45-49 days ago → last was 45 days ago
        // 45 / 10 = 4.5× → LetsRefocus
        let mut data = create_test_health_data(None, 50, 10, 5);
        data.completed_action_dates = vec![
            now - Duration::days(49),
            now - Duration::days(48),
            now - Duration::days(47),
            now - Duration::days(46),
            now - Duration::days(45),
        ];
        assert_eq!(compute_health(&data), Health::LetsRefocus);
    }
}
