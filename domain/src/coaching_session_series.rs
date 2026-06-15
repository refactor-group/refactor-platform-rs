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
use crate::gateway::tiptap::TiptapDocument;
use crate::Id;
use chrono::NaiveDateTime;
use entity_api::coaching_session_series;
use log::warn;
use sea_orm::{DatabaseConnection, TransactionTrait};
use serde::{Deserialize, Serialize};
use service::config::Config;

pub use coaching_session::{Frequency, Recurrence, RecurrenceError};
pub use entity::coaching_session_series::Model;
pub use entity_api::coaching_session_series::find_by_id;

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

/// List every series owned by the given coaching relationship.
pub async fn find_by_relationship(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(coaching_session_series::find_by_relationship(db, coaching_relationship_id).await?)
}

/// Replace the rule on an existing series and re-materialize its future
/// sessions. Future sessions are deleted unconditionally — notes, goal
/// links, recordings, and transcriptions cascade via the FK; Tiptap
/// documents are cleaned up best-effort after the DB transaction commits.
///
pub async fn reschedule(
    db: &DatabaseConnection,
    config: &Config,
    series_id: Id,
    coach_id: Id,
    new_start_at: NaiveDateTime,
    new_recurrence: Recurrence,
    new_requested_duration: Option<Duration>,
) -> Result<(Model, Vec<coaching_sessions::Model>), Error> {
    // Validate new inputs before any DB write. A reschedule only ever touches
    // future sessions (past sessions are left untouched), so a past
    // `new_start_at` would re-materialize the past on top of surviving history
    // — reject it up front rather than corrupt the timeline.
    let now_naive = chrono::Utc::now().naive_utc();
    if new_start_at < now_naive {
        return Err(RecurrenceError::StartAtInPast.into());
    }

    let new_dates = coaching_session::expand_recurrence(new_start_at, &new_recurrence)?;

    // Resolve the duration for the re-materialized sessions. A reschedule moves
    // the meetings; it must not silently restretch them. So when the caller
    // omits a duration, reuse the value persisted on the existing series rule
    // rather than re-deriving the coach's *current* default (which may have
    // changed since the series was created). An explicit request still wins.
    let resolved_duration = match new_requested_duration {
        Some(duration) => duration,
        None => {
            let existing = coaching_session_series::find_by_id(db, series_id).await?;
            let existing_rule: SeriesRule = serde_json::from_value(existing.rule)?;
            Duration::from_minutes_unchecked(existing_rule.duration_minutes)
        }
    };

    let new_rule = SeriesRule {
        start_at: new_start_at,
        recurrence: new_recurrence,
        duration_minutes: resolved_duration.minutes(),
    };
    let new_rule = serde_json::to_value(&new_rule)?;

    let future_sessions =
        entity_api::coaching_session::find_future_sessions_by_series_id(db, series_id, now_naive)
            .await?;

    // Snapshot Tiptap docs to clean up *after* the DB txn commits.
    let doc_names_to_cleanup: Vec<String> = future_sessions
        .iter()
        .filter_map(|s| s.collab_document_name.clone())
        .collect();
    let future_ids: Vec<Id> = future_sessions.iter().map(|s| s.id).collect();

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    for id in &future_ids {
        entity_api::coaching_session::acquire_advisory_lock(&txn, *id).await?;
    }

    entity_api::coaching_session::bulk_delete_by_ids(&txn, &future_ids).await?;

    let updated_series = coaching_session_series::update_rule(&txn, series_id, new_rule).await?;

    let new_sessions = coaching_session::bulk_create_recurring(
        &txn,
        updated_series.coaching_relationship_id,
        coach_id,
        updated_series.id,
        new_dates,
        Some(resolved_duration),
    )
    .await?;

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    cleanup_orphaned_docs(config, series_id, "reschedule", &doc_names_to_cleanup).await;

    Ok((updated_series, new_sessions))
}

/// Delete the series row and its future sessions. Past sessions survive as
/// orphan one-offs: the FK's `ON DELETE SET NULL` clears
/// `coaching_session_series_id` for every row that the explicit future
/// delete didn't touch, so their notes / meeting URLs / collab docs stay
/// intact.
///
pub async fn delete_with_future_sessions(
    db: &DatabaseConnection,
    config: &Config,
    series_id: Id,
) -> Result<(), Error> {
    let now_naive = chrono::Utc::now().naive_utc();
    let future_sessions =
        entity_api::coaching_session::find_future_sessions_by_series_id(db, series_id, now_naive)
            .await?;

    let doc_names_to_cleanup: Vec<String> = future_sessions
        .iter()
        .filter_map(|s| s.collab_document_name.clone())
        .collect();
    let future_ids: Vec<Id> = future_sessions.iter().map(|s| s.id).collect();

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    for id in &future_ids {
        entity_api::coaching_session::acquire_advisory_lock(&txn, *id).await?;
    }

    entity_api::coaching_session::bulk_delete_by_ids(&txn, &future_ids).await?;

    coaching_session_series::delete(&txn, series_id).await?;

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    cleanup_orphaned_docs(config, series_id, "delete", &doc_names_to_cleanup).await;

    Ok(())
}

async fn cleanup_orphaned_docs(
    config: &Config,
    series_id: Id,
    operation: &str,
    doc_names: &[String],
) {
    if doc_names.is_empty() {
        return;
    }
    match TiptapDocument::new(config).await {
        Ok(tiptap) => {
            for name in doc_names {
                if let Err(err) = tiptap.delete(name).await {
                    warn!(
                        "Tiptap cleanup failed for orphaned doc {name:?} after series \
                         {operation} {series_id}: {err}"
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                "Could not construct Tiptap client to clean up {} orphaned doc(s) \
                 after series {operation} {series_id}: {err}",
                doc_names.len()
            );
        }
    }
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
        // Relative to now so `reschedule`'s past-guard (and date rollover)
        // can't make this flaky; a fixed calendar date eventually goes stale.
        (chrono::Utc::now() + chrono::Duration::days(7)).naive_utc()
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
            title: None,
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

    /// Happy-path reschedule over unhydrated future sessions, with the duration
    /// omitted: validates inputs, reuses the duration persisted on the existing
    /// series rule (no coach lookup), deletes the old future rows, rewrites the
    /// rule, and bulk-inserts the new schedule — all in one transaction. Tiptap
    /// is never contacted because no future session has a collab doc.
    #[tokio::test]
    async fn reschedule_replaces_future_unhydrated_sessions() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let coach_id = Id::new_v4();
        let series_id = Id::new_v4();
        let now = chrono::Utc::now();

        // The series as it currently exists: its rule carries the duration the
        // None-path reschedule must reuse instead of re-deriving a default.
        let existing_series = Model {
            id: series_id,
            coaching_relationship_id: relationship_id,
            rule: serde_json::to_value(SeriesRule {
                start_at: start(),
                recurrence: weekly_rule_count(3),
                duration_minutes: 60,
            })?,
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let make_session = |date: NaiveDateTime, doc: Option<String>| coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: Some(series_id),
            collab_document_name: doc,
            date,
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
        };

        // Two future sessions, neither hydrated, neither carrying a Tiptap doc.
        let future_sessions = vec![
            make_session(start() + chrono::Duration::days(7), None),
            make_session(start() + chrono::Duration::days(14), None),
        ];

        // The series row as returned by update_rule's internal find_by_id, then again
        // by the UPDATE ... RETURNING.
        let updated_series = Model {
            id: series_id,
            coaching_relationship_id: relationship_id,
            rule: serde_json::json!({}),
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        // New schedule: 3 weekly occurrences starting at start().
        let new_sessions = vec![
            make_session(start(), None),
            make_session(start() + chrono::Duration::days(7), None),
            make_session(start() + chrono::Duration::days(14), None),
        ];

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // 1. duration omitted → find_by_id on the series to read its stored rule
            .append_query_results(vec![vec![existing_series.clone()]])
            // 2. find_future_sessions_by_series_id → 2 future rows
            .append_query_results(vec![future_sessions.clone()])
            // 3. BEGIN
            .append_exec_results(vec![MockExecResult { last_insert_id: 0, rows_affected: 1 }])
            // 4. acquire_advisory_lock × 2 (one exec per session)
            .append_exec_results(vec![MockExecResult { last_insert_id: 0, rows_affected: 1 }])
            .append_exec_results(vec![MockExecResult { last_insert_id: 0, rows_affected: 1 }])
            // 5. bulk_delete_by_ids → DELETE
            .append_exec_results(vec![MockExecResult { last_insert_id: 0, rows_affected: 2 }])
            // 6. update_rule → internal find_by_id (SELECT) → UPDATE ... RETURNING
            .append_query_results(vec![vec![updated_series.clone()]])
            .append_query_results(vec![vec![updated_series.clone()]])
            // 7. bulk_create_recurring → INSERT ... RETURNING
            .append_query_results(vec![new_sessions.clone()])
            // 8. COMMIT
            .append_exec_results(vec![MockExecResult { last_insert_id: 0, rows_affected: 1 }])
            .into_connection();

        let (returned_series, returned_sessions) = reschedule(
            &db,
            &test_config(),
            series_id,
            coach_id,
            start(),
            weekly_rule_count(3),
            None,
        )
        .await?;

        assert_eq!(returned_series.id, series_id);
        assert_eq!(returned_sessions.len(), 3);
        assert!(returned_sessions
            .iter()
            .all(|s| s.coaching_session_series_id == Some(series_id)));
        Ok(())
    }

    /// A reschedule with a `start_at` in the past is rejected before any DB
    /// access — re-materializing the past on top of surviving past sessions
    /// would corrupt the series timeline.
    #[tokio::test]
    async fn reschedule_rejects_past_start_at() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let past = NaiveDate::from_ymd_opt(2000, 1, 1)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();

        let result = reschedule(
            &db,
            &test_config(),
            Id::new_v4(),
            Id::new_v4(),
            past,
            weekly_rule_count(3),
            None,
        )
        .await;

        let err = result.expect_err("a past start_at must be rejected");
        assert!(
            matches!(
                err.error_kind,
                crate::error::DomainErrorKind::Validation(ref m) if m.contains("past")
            ),
            "expected a past-start validation error, got {err:?}"
        );
    }

    /// Series delete: future sessions are loaded, locked, bulk-deleted, and
    /// then the series row itself is removed. Past sessions are not touched.
    /// Tiptap is never reached because no future session has a
    /// collab doc.
    #[tokio::test]
    async fn delete_with_future_sessions_clears_future_and_series_rows() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let series_id = Id::new_v4();
        let now = chrono::Utc::now();

        let make_session = |date: NaiveDateTime| coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: Some(series_id),
            collab_document_name: None,
            date,
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
        };

        let future_sessions = vec![
            make_session(start() + chrono::Duration::days(7)),
            make_session(start() + chrono::Duration::days(14)),
        ];

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // find_future_sessions_by_series_id → 2 future rows
            .append_query_results(vec![future_sessions.clone()])
            // BEGIN
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // acquire_advisory_lock × 2
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // bulk_delete_by_ids → DELETE
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 2,
            }])
            // coaching_session_series::delete → DELETE
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // COMMIT
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        delete_with_future_sessions(&db, &test_config(), series_id).await?;
        Ok(())
    }

    fn test_config() -> Config {
        Config::from_args([
            "test",
            "--tiptap-auth-key=test-auth-key",
            "--tiptap-url=http://127.0.0.1:0",
        ])
    }
}
