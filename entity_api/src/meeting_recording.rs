use super::error::{EntityApiErrorKind, Error};
use entity::meeting_recording::{ActiveModel, Column, Entity, MeetingRecordingStatus, Model};
use entity::Id;
use log::debug;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, IntoActiveModel, Order, QueryOrder, QuerySelect, TransactionError,
    TransactionTrait, TryIntoModel,
};

const TERMINAL_RECORDING_STATUSES: &[MeetingRecordingStatus] = &[
    MeetingRecordingStatus::Completed,
    MeetingRecordingStatus::Failed,
    MeetingRecordingStatus::Cancelled,
];

/// Creates a new meeting recording record
pub async fn create(db: &DatabaseConnection, model: Model) -> Result<Model, Error> {
    debug!(
        "Creating meeting recording for coaching_session_id: {}",
        model.coaching_session_id
    );

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        coaching_session_id: Set(model.coaching_session_id),
        bot_id: Set(model.bot_id),
        status: Set(model.status),
        video_url: Set(model.video_url),
        audio_url: Set(model.audio_url),
        duration_seconds: Set(model.duration_seconds),
        started_at: Set(model.started_at),
        ended_at: Set(model.ended_at),
        error_message: Set(model.error_message),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Returns the most recent recording for a coaching session (by `created_at DESC`)
pub async fn find_latest_by_coaching_session(
    db: &DatabaseConnection,
    session_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingSessionId.eq(session_id))
        .order_by(Column::CreatedAt, Order::Desc)
        .one(db)
        .await?)
}

/// Finds a recording by its primary key
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Option<Model>, Error> {
    Ok(Entity::find_by_id(id).one(db).await?)
}

/// Finds a recording by Recall.ai bot ID — used by webhook handlers
pub async fn find_by_bot_id(db: &DatabaseConnection, bot_id: &str) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::BotId.eq(bot_id))
        .one(db)
        .await?)
}

/// Atomically transitions a recording to `Completed` along with `ended_at` and
/// `duration_seconds` in a single transaction. Returns `true` if the transition
/// succeeded; the caller won the race and should proceed with transcription. Returns
/// `false` if the recording was already terminal and the caller should skip.
///
/// `ended_at` is written when not already set; `duration_seconds` is auto-derived
/// from `started_at` + `ended_at` when both are known. Folding these writes inside
/// the same transaction as the status flip means a `Completed` row can never end up
/// with `NULL` timestamps due to a partial-failure window.
pub async fn try_claim_completed(db: &DatabaseConnection, id: Id) -> Result<bool, Error> {
    db.transaction::<_, bool, Error>(|txn| {
        Box::pin(async move {
            let Some(model) = Entity::find_by_id(id)
                .lock_exclusive()
                .one(txn)
                .await?
                .filter(|m| !TERMINAL_RECORDING_STATUSES.contains(&m.status))
            else {
                return Ok(false);
            };

            let now: DateTimeWithTimeZone = chrono::Utc::now().into();
            let new_ended_at = model.ended_at.or(Some(now));
            let new_duration_seconds = model
                .duration_seconds
                .or_else(|| derive_duration_seconds(model.started_at, new_ended_at));

            ActiveModel {
                status: Set(MeetingRecordingStatus::Completed),
                ended_at: Set(new_ended_at),
                duration_seconds: Set(new_duration_seconds),
                updated_at: Set(now),
                ..model.into_active_model()
            }
            .update(txn)
            .await?;

            Ok(true)
        })
    })
    .await
    .map_err(|e| match e {
        TransactionError::Connection(db_err) => db_err.into(),
        TransactionError::Transaction(err) => err,
    })
}

/// Derive `duration_seconds` from `started_at` + `ended_at` when both are known.
/// Returns `None` if either timestamp is missing, if the result would be non-positive
/// (clock skew), or if it would overflow `i32` (≈68 years — physically impossible but
/// guarded explicitly rather than silently truncated).
fn derive_duration_seconds(
    started_at: Option<DateTimeWithTimeZone>,
    ended_at: Option<DateTimeWithTimeZone>,
) -> Option<i32> {
    match (started_at, ended_at) {
        (Some(start), Some(end)) => {
            let secs = (end - start).num_seconds();
            i32::try_from(secs).ok().filter(|&s| s > 0)
        }
        _ => None,
    }
}

/// Optional artifact fields to set when updating a recording's status.
///
/// Fields follow a preserve-or-overwrite pattern: `None` keeps the existing value,
/// `Some(x)` overwrites it. Fields cannot be cleared back to `None` via this struct.
#[derive(Default)]
pub struct RecordingArtifacts {
    pub video_url: Option<String>,
    pub audio_url: Option<String>,
    pub duration_seconds: Option<i32>,
    pub started_at: Option<DateTimeWithTimeZone>,
    pub ended_at: Option<DateTimeWithTimeZone>,
    pub error_message: Option<String>,
}

/// Updates recording status and optional artifact fields.
pub async fn update_status(
    db: &DatabaseConnection,
    id: Id,
    status: MeetingRecordingStatus,
    artifacts: RecordingArtifacts,
) -> Result<Model, Error> {
    let existing = Entity::find_by_id(id).one(db).await?.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })?;

    debug!("Updating meeting recording status: {id}");

    let new_started_at = artifacts.started_at.or(existing.started_at);
    let new_ended_at = artifacts.ended_at.or(existing.ended_at);

    // Auto-derive duration_seconds from start/end whenever both are known and no
    // explicit value is supplied. Keeps the field consistent across every transition
    // path (bot.done, recording.failed, user-cancel, etc.) without duplicating math.
    let new_duration_seconds = artifacts
        .duration_seconds
        .or(existing.duration_seconds)
        .or_else(|| derive_duration_seconds(new_started_at, new_ended_at));

    let active_model = ActiveModel {
        id: Unchanged(existing.id),
        coaching_session_id: Unchanged(existing.coaching_session_id),
        bot_id: Unchanged(existing.bot_id),
        status: Set(status),
        video_url: Set(artifacts.video_url.or(existing.video_url)),
        audio_url: Set(artifacts.audio_url.or(existing.audio_url)),
        duration_seconds: Set(new_duration_seconds),
        started_at: Set(new_started_at),
        ended_at: Set(new_ended_at),
        error_message: Set(artifacts.error_message.or(existing.error_message)),
        created_at: Unchanged(existing.created_at),
        updated_at: Set(chrono::Utc::now().into()),
    };

    Ok(active_model.update(db).await?.try_into_model()?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction, Value};

    /// Locate a column's bind index by parsing the UPDATE SET clause directly.
    /// Robust against ActiveModel field reordering or SeaORM changing SET-bind ordering.
    fn bind_index_for_column(sql: &str, column: &str) -> usize {
        let needle = format!(r#""{column}" = $"#);
        let start = sql
            .find(&needle)
            .unwrap_or_else(|| panic!("column {column:?} not found in SQL: {sql}"));
        let after = &sql[start + needle.len()..];
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        let one_based: usize = after[..end]
            .parse()
            .unwrap_or_else(|_| panic!("could not parse bind position for {column} in: {sql}"));
        one_based - 1
    }

    /// Extract the `duration_seconds` bind from the UPDATE statement in a captured txn log.
    fn duration_seconds_bind_in_update(log: &[Transaction]) -> Option<i32> {
        for txn in log {
            for stmt in txn.statements() {
                if stmt.sql.starts_with("UPDATE ") {
                    let idx = bind_index_for_column(&stmt.sql, "duration_seconds");
                    let binds = &stmt.values.as_ref().expect("update has binds").0;
                    return match &binds[idx] {
                        Value::Int(opt) => *opt,
                        other => panic!("bind for duration_seconds was not Int: {other:?}"),
                    };
                }
            }
        }
        panic!("no UPDATE statement found in transaction log");
    }

    fn test_model() -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            bot_id: "recall-bot-abc123".to_string(),
            status: MeetingRecordingStatus::Pending,
            video_url: None,
            audio_url: None,
            duration_seconds: None,
            started_at: None,
            ended_at: None,
            error_message: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_returns_a_new_meeting_recording() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = create(&db, model.clone()).await?;

        assert_eq!(result.coaching_session_id, model.coaching_session_id);
        assert_eq!(result.bot_id, model.bot_id);
        assert_eq!(result.status, MeetingRecordingStatus::Pending);

        Ok(())
    }

    #[tokio::test]
    async fn find_latest_by_coaching_session_returns_none_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_latest_by_coaching_session(&db, Id::new_v4()).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn find_latest_by_coaching_session_returns_model_when_found() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = find_latest_by_coaching_session(&db, model.coaching_session_id).await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().bot_id, model.bot_id);
        Ok(())
    }

    #[tokio::test]
    async fn find_by_bot_id_returns_none_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_by_bot_id(&db, "nonexistent-bot").await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn find_by_bot_id_returns_model_when_found() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = find_by_bot_id(&db, &model.bot_id).await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().bot_id, model.bot_id);
        Ok(())
    }

    #[tokio::test]
    async fn update_status_updates_recording_status() -> Result<(), Error> {
        let model = test_model();
        let mut updated = model.clone();
        updated.status = MeetingRecordingStatus::Recording;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .append_query_results(vec![vec![updated.clone()]])
            .into_connection();

        let result = update_status(
            &db,
            model.id,
            MeetingRecordingStatus::Recording,
            RecordingArtifacts::default(),
        )
        .await?;

        assert_eq!(result.status, MeetingRecordingStatus::Recording);
        Ok(())
    }

    #[tokio::test]
    async fn update_status_auto_derives_duration_seconds_from_timestamps() -> Result<(), Error> {
        let start = chrono::Utc::now() - chrono::Duration::seconds(125);
        let end = chrono::Utc::now();
        let mut existing = test_model();
        existing.started_at = Some(start.into());
        existing.ended_at = None;
        existing.duration_seconds = None;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]])
            .append_query_results(vec![vec![existing.clone()]])
            .into_connection();

        update_status(
            &db,
            existing.id,
            MeetingRecordingStatus::Processing,
            RecordingArtifacts {
                ended_at: Some(end.into()),
                ..Default::default()
            },
        )
        .await?;

        // Inspect the actual UPDATE bind, not the mock's return value.
        let bound = duration_seconds_bind_in_update(&db.into_transaction_log())
            .expect("auto-derive should have produced Some(_)");
        // Allow ±1s slack for the `chrono::Utc::now()` capture above.
        assert!(
            (124..=126).contains(&bound),
            "expected duration ~125s, got {bound}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn update_status_does_not_overwrite_explicit_duration_seconds() -> Result<(), Error> {
        let mut existing = test_model();
        existing.started_at = Some((chrono::Utc::now() - chrono::Duration::seconds(60)).into());
        existing.duration_seconds = Some(999); // pre-set, must be preserved

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]])
            .append_query_results(vec![vec![existing.clone()]])
            .into_connection();

        update_status(
            &db,
            existing.id,
            MeetingRecordingStatus::Processing,
            RecordingArtifacts {
                ended_at: Some(chrono::Utc::now().into()),
                ..Default::default()
            },
        )
        .await?;

        let bound = duration_seconds_bind_in_update(&db.into_transaction_log());
        assert_eq!(
            bound,
            Some(999),
            "existing duration_seconds=999 must be preserved, not overwritten by derived ~60s"
        );
        Ok(())
    }

    #[tokio::test]
    async fn update_status_skips_duration_when_started_at_unknown() -> Result<(), Error> {
        let mut existing = test_model();
        existing.started_at = None; // missing → cannot derive
        existing.ended_at = None;
        existing.duration_seconds = None;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]])
            .append_query_results(vec![vec![existing.clone()]])
            .into_connection();

        // ended_at supplied, but started_at is still None → no derivation possible.
        update_status(
            &db,
            existing.id,
            MeetingRecordingStatus::Processing,
            RecordingArtifacts {
                ended_at: Some(chrono::Utc::now().into()),
                ..Default::default()
            },
        )
        .await?;

        let bound = duration_seconds_bind_in_update(&db.into_transaction_log());
        assert_eq!(bound, None, "no derivation possible without started_at");
        Ok(())
    }

    #[tokio::test]
    async fn update_status_skips_negative_duration_from_clock_skew() -> Result<(), Error> {
        let mut existing = test_model();
        // Pathological: end is *before* start (clock skew, manual data, etc.).
        existing.started_at = Some(chrono::Utc::now().into());
        existing.ended_at = None;
        existing.duration_seconds = None;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]])
            .append_query_results(vec![vec![existing.clone()]])
            .into_connection();

        let earlier = chrono::Utc::now() - chrono::Duration::seconds(30);
        update_status(
            &db,
            existing.id,
            MeetingRecordingStatus::Processing,
            RecordingArtifacts {
                ended_at: Some(earlier.into()),
                ..Default::default()
            },
        )
        .await?;

        let bound = duration_seconds_bind_in_update(&db.into_transaction_log());
        assert_eq!(
            bound, None,
            "negative-duration arithmetic (end<start) should not store a value"
        );
        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_error_when_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = update_status(
            &db,
            Id::new_v4(),
            MeetingRecordingStatus::Failed,
            RecordingArtifacts::default(),
        )
        .await;

        assert!(result.is_err());
    }

    fn test_model_with_status(status: MeetingRecordingStatus) -> Model {
        let mut m = test_model();
        m.status = status;
        m
    }

    #[tokio::test]
    async fn try_claim_completed_returns_true_when_non_terminal() -> Result<(), Error> {
        let existing = test_model_with_status(MeetingRecordingStatus::Processing);
        let id = existing.id;
        let mut after_update = existing.clone();
        after_update.status = MeetingRecordingStatus::Completed;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing]])
            .append_query_results(vec![vec![after_update]])
            .into_connection();

        let result = try_claim_completed(&db, id).await?;
        assert!(result);
        Ok(())
    }

    #[tokio::test]
    async fn try_claim_completed_returns_false_when_already_terminal() -> Result<(), Error> {
        let existing = test_model_with_status(MeetingRecordingStatus::Completed);
        let id = existing.id;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing]])
            .into_connection();

        let result = try_claim_completed(&db, id).await?;
        assert!(!result);
        Ok(())
    }

    #[tokio::test]
    async fn try_claim_completed_returns_false_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = try_claim_completed(&db, Id::new_v4()).await?;
        assert!(!result);
        Ok(())
    }

    #[tokio::test]
    async fn try_claim_completed_writes_ended_at_and_derives_duration_atomically(
    ) -> Result<(), Error> {
        let start = chrono::Utc::now() - chrono::Duration::seconds(300);
        let mut existing = test_model_with_status(MeetingRecordingStatus::Processing);
        existing.started_at = Some(start.into());
        existing.ended_at = None;
        existing.duration_seconds = None;
        let id = existing.id;

        let before = chrono::Utc::now();
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]])
            .append_query_results(vec![vec![existing]])
            .into_connection();

        let result = try_claim_completed(&db, id).await?;
        assert!(result, "claim should succeed");

        let log = db.into_transaction_log();
        let after = chrono::Utc::now();

        // ended_at written in the same UPDATE that flipped status — no best-effort window.
        let ended = match log.iter().find_map(|t| {
            t.statements()
                .iter()
                .find(|s| s.sql.starts_with("UPDATE "))
                .cloned()
        }) {
            Some(stmt) => {
                let idx = bind_index_for_column(&stmt.sql, "ended_at");
                match &stmt.values.as_ref().unwrap().0[idx] {
                    Value::ChronoDateTimeWithTimeZone(opt) => opt.as_deref().copied(),
                    other => panic!("bind for ended_at not a timestamp: {other:?}"),
                }
            }
            None => panic!("no UPDATE in log"),
        };
        let ended = ended.expect("ended_at must be set by atomic claim");
        assert!(
            ended.to_utc() >= before && ended.to_utc() <= after,
            "ended_at should be freshly captured"
        );

        // duration_seconds derived from started_at + ended_at, ≈300s within ±1s slack.
        let duration =
            duration_seconds_bind_in_update(&log).expect("duration_seconds must be derived");
        assert!(
            (299..=301).contains(&duration),
            "expected ~300s, got {duration}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn try_claim_completed_preserves_existing_ended_at() -> Result<(), Error> {
        let preset_end: DateTimeWithTimeZone =
            (chrono::Utc::now() - chrono::Duration::seconds(30)).into();
        let mut existing = test_model_with_status(MeetingRecordingStatus::Processing);
        existing.ended_at = Some(preset_end); // set by an earlier bot.done transition
        let id = existing.id;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]])
            .append_query_results(vec![vec![existing]])
            .into_connection();

        try_claim_completed(&db, id).await?;

        let log = db.into_transaction_log();
        let stmt = log
            .iter()
            .find_map(|t| {
                t.statements()
                    .iter()
                    .find(|s| s.sql.starts_with("UPDATE "))
                    .cloned()
            })
            .expect("no UPDATE in log");
        let idx = bind_index_for_column(&stmt.sql, "ended_at");
        let bound = match &stmt.values.as_ref().unwrap().0[idx] {
            Value::ChronoDateTimeWithTimeZone(opt) => opt.as_deref().copied(),
            other => panic!("bind for ended_at not a timestamp: {other:?}"),
        };
        assert_eq!(
            bound,
            Some(preset_end),
            "preset ended_at must be preserved, not overwritten by Utc::now()"
        );
        Ok(())
    }
}
