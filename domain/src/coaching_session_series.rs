//! Domain orchestration for `coaching_session_series` — the entity that owns
//! a recurrence rule and groups the materialized `coaching_sessions` rows
//! created from it.
//!
//! This module owns the JSONB rule serialization: callers interact with the
//! typed [`SeriesRule`] struct, never with `serde_json::Value` directly.

use crate::coaching_session;
use crate::coaching_sessions;
use crate::duration::Duration;
use crate::error::Error;
use crate::Id;
use chrono::NaiveDateTime;
use entity_api::coaching_session_series;
use sea_orm::{DatabaseConnection, TransactionTrait};
use serde::{Deserialize, Serialize};

pub use coaching_session::{Frequency, Recurrence, RecurrenceError};
pub use entity::coaching_session_series::Model;

/// Typed shape of the JSONB `rule` column on `coaching_session_series`.
/// Stored at create time and re-read at reschedule time, which is why
/// `duration_minutes` is captured here — a coach who updates their default
/// duration between create and reschedule shouldn't see the existing series
/// flip durations under them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesRule {
    pub start_at: NaiveDateTime,
    pub recurrence: Recurrence,
    pub duration_minutes: i16,
}

/// Create a series and materialize its sessions in a single transaction.
///
/// 1. Expands and validates the recurrence rule.
/// 2. Resolves the coach's effective duration via the defaulting cascade.
/// 3. Inserts the series row with the resolved rule (including the resolved
///    `duration_minutes`) serialized to JSONB.
/// 4. Bulk-inserts the materialized sessions linked back to the series via
///    `coaching_session_series_id`.
///
/// Returns `(series, sessions)`. Tiptap docs, meeting URLs, and goal links
/// are NOT created here — those run lazily on first read via
/// `coaching_session::ensure_hydrated`.
pub async fn create_with_sessions(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
    coach_id: Id,
    created_by_user_id: Id,
    start_at: NaiveDateTime,
    recurrence: Recurrence,
    requested_duration: Option<Duration>,
) -> Result<(Model, Vec<coaching_sessions::Model>), Error> {
    let dates = coaching_session::expand_recurrence(start_at, &recurrence)?;
    let resolved_duration =
        entity_api::coaching_session::resolve_duration(db, coach_id, requested_duration).await?;

    let rule = SeriesRule {
        start_at,
        recurrence,
        duration_minutes: resolved_duration.minutes(),
    };
    let rule_json = serde_json::to_value(&rule)?;

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    let series_input = Model {
        id: Id::nil(),
        coaching_relationship_id,
        rule: rule_json,
        created_by_user_id,
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
    };
    let series = coaching_session_series::create(&txn, series_input).await?;

    let sessions = coaching_session::bulk_create_recurring(
        &txn,
        coaching_relationship_id,
        coach_id,
        series.id,
        dates,
        Some(resolved_duration),
    )
    .await?;

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    Ok((series, sessions))
}

/// Read a series row by id alongside every coaching_session linked to it,
/// ordered by `date ASC`.
pub async fn find_by_id_with_sessions(
    db: &DatabaseConnection,
    id: Id,
) -> Result<(Model, Vec<coaching_sessions::Model>), Error> {
    let series = coaching_session_series::find_by_id(db, id).await?;
    let sessions = coaching_session::find_by_series_id(db, id).await?;
    Ok((series, sessions))
}

/// List every series owned by the given coaching relationship.
pub async fn find_by_relationship(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(coaching_session_series::find_by_relationship(db, coaching_relationship_id).await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use crate::coaching_session::Frequency;
    use chrono::NaiveDate;
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

    fn weekly_rule_count(n: u32) -> Recurrence {
        Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: Some(n),
            until: None,
        }
    }

    fn start() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 6, 15)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
    }

    #[tokio::test]
    async fn create_with_sessions_inserts_series_and_sessions_in_one_transaction(
    ) -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let coach_id = Id::new_v4();
        let created_by = Id::new_v4();
        let now = chrono::Utc::now();
        let rule = weekly_rule_count(3);

        let series = Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            rule: serde_json::json!({}),
            created_by_user_id: created_by,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let coach = entity::users::Model {
            id: coach_id,
            email: "coach@example.com".into(),
            first_name: "Coach".into(),
            last_name: "One".into(),
            display_name: None,
            password: None,
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".into(),
            default_coaching_session_duration_minutes: 60,
            role: Default::default(),
            roles: vec![],
            invite_status: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let make_session = |date: NaiveDateTime| coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: Some(series.id),
            collab_document_name: None,
            date,
            duration_minutes: 60,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
        };

        let expected_sessions = vec![
            make_session(start()),
            make_session(start() + chrono::Duration::days(7)),
            make_session(start() + chrono::Duration::days(14)),
        ];

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<(entity::users::Model, Option<entity::user_roles::Model>), _, _>(
                vec![vec![(coach, None)]],
            )
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![expected_sessions.clone()])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let (returned_series, returned_sessions) = create_with_sessions(
            &db,
            relationship_id,
            coach_id,
            created_by,
            start(),
            rule,
            None,
        )
        .await?;

        assert_eq!(returned_series.id, series.id);
        assert_eq!(returned_sessions.len(), 3);
        assert!(returned_sessions
            .iter()
            .all(|s| s.coaching_session_series_id == Some(series.id)));
        Ok(())
    }

    #[tokio::test]
    async fn create_with_sessions_rejects_invalid_recurrence() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let bad_rule = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: Some(3),
            until: Some(start() + chrono::Duration::days(21)),
        };
        let result = create_with_sessions(
            &db,
            Id::new_v4(),
            Id::new_v4(),
            Id::new_v4(),
            start(),
            bad_rule,
            Some(Duration::default()),
        )
        .await;
        assert!(result.is_err());
    }
}
