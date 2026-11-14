//! What stamping notice leaves in the table, against a real SQL engine.

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use entity_api::coaching_session_notice::{self, Model};
use sea_orm::DatabaseConnection;

use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_coaching_session, seed_user, within_time_limit,
};
use crate::Id;

fn at(hour: u32) -> DateTime<FixedOffset> {
    Utc.with_ymd_and_hms(2026, 12, 1, hour, 0, 0)
        .single()
        .expect("valid test time")
        .fixed_offset()
}

/// A database with the notices table's unique key, holding one session and its participant.
async fn session() -> (DatabaseConnection, Id, Id) {
    let db = database().await;
    mirror_migration_defaults(&db).await;
    let (session_id, user_id) = seed_coaching_session(&db).await;
    (db, session_id, user_id)
}

async fn stamp(db: &DatabaseConnection, session_id: Id, user_ids: &[Id], when: u32) {
    coaching_session_notice::stamp(db, session_id, user_ids, at(when))
        .await
        .expect("the notice stamps");
}

async fn rows(db: &DatabaseConnection, session_id: Id) -> Vec<Model> {
    coaching_session_notice::find_by_coaching_session(db, session_id)
        .await
        .expect("the notices read")
}

fn row_for(rows: &[Model], user_id: Id) -> &Model {
    rows.iter()
        .find(|row| row.user_id == user_id)
        .expect("the user has a notice row")
}

#[tokio::test]
async fn stamping_two_users_records_one_notice_each() {
    within_time_limit(async {
        let (db, session_id, coach_id) = session().await;
        let coachee_id = seed_user(&db).await;

        stamp(&db, session_id, &[coach_id, coachee_id], 9).await;

        let rows = rows(&db, session_id).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(row_for(&rows, coach_id).notice_given_at, at(9));
        assert_eq!(row_for(&rows, coachee_id).notice_given_at, at(9));
    })
    .await;
}

#[tokio::test]
async fn restamping_a_user_replaces_only_their_notice() {
    within_time_limit(async {
        let (db, session_id, coach_id) = session().await;
        let coachee_id = seed_user(&db).await;

        stamp(&db, session_id, &[coach_id, coachee_id], 9).await;
        stamp(&db, session_id, &[coachee_id], 11).await;

        let rows = rows(&db, session_id).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(row_for(&rows, coachee_id).notice_given_at, at(11));
        assert_eq!(row_for(&rows, coach_id).notice_given_at, at(9));
    })
    .await;
}

#[tokio::test]
async fn stamping_the_same_user_twice_in_one_call_records_one_notice() {
    within_time_limit(async {
        let (db, session_id, user_id) = session().await;

        stamp(&db, session_id, &[user_id, user_id], 9).await;

        let rows = rows(&db, session_id).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(row_for(&rows, user_id).notice_given_at, at(9));
    })
    .await;
}

#[tokio::test]
async fn stamping_no_users_records_nothing() {
    within_time_limit(async {
        let (db, session_id, _) = session().await;

        stamp(&db, session_id, &[], 9).await;

        assert!(rows(&db, session_id).await.is_empty());
    })
    .await;
}
