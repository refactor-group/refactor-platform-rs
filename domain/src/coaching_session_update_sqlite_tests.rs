//! What a session edit records as an unannounced reschedule, and what clears it, against a
//! real SQL engine.

use chrono::{NaiveDateTime, TimeDelta, Timelike, Utc};
use entity::{coaching_sessions, Id};
use entity_api::{coaching_session_pending_reschedule, mutate};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set, Value};

use crate::test_utils::sqlite::{database, seed_coaching_session, within_time_limit};

#[derive(Debug)]
struct Params(mutate::UpdateMap);

impl mutate::IntoUpdateMap for Params {
    fn into_update_map(self) -> mutate::UpdateMap {
        self.0
    }
}

fn set(column: &str, value: Value) -> Params {
    let mut map = mutate::UpdateMap::new();
    map.insert(column.into(), Some(value));
    Params(map)
}

fn date(value: NaiveDateTime) -> Params {
    set("date", Value::ChronoDateTime(Some(value)))
}

fn duration(minutes: i16) -> Params {
    set("duration_minutes", Value::SmallInt(Some(minutes)))
}

fn title(value: &str) -> Params {
    set("title", Value::String(Some(value.to_string())))
}

/// A session a week out, 60 minutes long; returns its id and start.
async fn upcoming_session(db: &DatabaseConnection) -> (Id, NaiveDateTime) {
    let (session_id, _) = seed_coaching_session(db).await;
    let start = (Utc::now() + TimeDelta::days(7))
        .naive_utc()
        .with_nanosecond(0)
        .expect("whole seconds");
    let mut session = coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session exists")
        .into_active_model();
    session.date = Set(start);
    session.update(db).await.expect("the session moves");
    (session_id, start)
}

async fn edit(db: &DatabaseConnection, session_id: Id, params: Params) -> coaching_sessions::Model {
    crate::coaching_session::update(db, session_id, params)
        .await
        .expect("the edit succeeds")
}

async fn pending(
    db: &DatabaseConnection,
    session_id: Id,
) -> Option<entity::coaching_session_pending_reschedules::Model> {
    coaching_session_pending_reschedule::find(db, session_id)
        .await
        .expect("the pending row reads")
}

#[tokio::test]
async fn moving_a_session_records_what_participants_were_last_told() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        let moved = edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        assert_eq!(moved.date, start + TimeDelta::hours(1));

        let row = pending(&db, session_id).await.expect("the move is pending");
        assert_eq!(row.announced_date, start);
        assert_eq!(row.announced_duration_minutes, 60);
        assert_eq!(row.revision, 0);
        assert_eq!(row.claim_id, None);
    })
    .await;
}

#[tokio::test]
async fn resizing_a_session_records_a_pending_change() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        edit(&db, session_id, duration(45)).await;

        let row = pending(&db, session_id)
            .await
            .expect("the resize is pending");
        assert_eq!(row.announced_date, start);
        assert_eq!(row.announced_duration_minutes, 60);
    })
    .await;
}

#[tokio::test]
async fn further_moves_keep_the_first_announced_schedule() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        edit(&db, session_id, date(start + TimeDelta::hours(2))).await;

        let row = pending(&db, session_id)
            .await
            .expect("the moves are pending");
        assert_eq!(row.announced_date, start);
        assert_eq!(row.revision, 1);
    })
    .await;
}

#[tokio::test]
async fn moving_back_to_the_announced_schedule_leaves_nothing_pending() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        edit(&db, session_id, date(start)).await;

        assert_eq!(pending(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_title_edit_records_nothing() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, _) = upcoming_session(&db).await;

        edit(&db, session_id, title("Renamed")).await;

        assert_eq!(pending(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn an_edit_that_changes_nothing_records_nothing() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        edit(&db, session_id, date(start)).await;

        assert_eq!(pending(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn recording_notice_clears_the_pending_change() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;
        let before = coaching_sessions::Entity::find_by_id(session_id)
            .one(&db)
            .await
            .expect("the session reads")
            .expect("the session exists")
            .notice_given_at;

        let sent = edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        let stamped = crate::coaching_session::record_notice(&db, &sent)
            .await
            .expect("notice records");

        assert!(stamped.notice_given_at > before);
        assert_eq!(pending(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn an_edit_after_the_email_was_read_stays_pending_from_what_was_sent() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        let sent = edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        edit(&db, session_id, date(start + TimeDelta::hours(2))).await;
        crate::coaching_session::record_announced(&db, &sent).await;

        let row = pending(&db, session_id)
            .await
            .expect("the newer edit stays pending");
        assert_eq!(row.announced_date, sent.date);
    })
    .await;
}
