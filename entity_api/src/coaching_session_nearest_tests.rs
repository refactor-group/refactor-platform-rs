//! Control flow of `find_nearest_by_user_with_includes`; which rows the queries
//! select is covered by the SQLite tier in `domain`.

use super::*;
use chrono::{NaiveDate, Utc};
use entity::Id;
use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

const SESSION_COLUMNS: &str = r#""coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."coaching_session_series_id", "coaching_sessions"."ical_sequence", "coaching_sessions"."ical_recurrence_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."title", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at", "coaching_sessions"."notice_given_at""#;

fn around() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 30)
        .unwrap()
        .and_hms_opt(14, 0, 0)
        .unwrap()
}

fn window(coaching_relationship_id: Id) -> NearestWindow {
    NearestWindow {
        coaching_relationship_id,
        around: around(),
        before: 2,
        after: 3,
    }
}

fn relationship(id: Id, coach_id: Id, coachee_id: Id) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id,
        organization_id: Id::new_v4(),
        coach_id,
        coachee_id,
        slug: "coach-coachee".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn session(coaching_relationship_id: Id, day: u32) -> Model {
    let now = Utc::now();
    Model {
        id: Id::new_v4(),
        coaching_relationship_id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(14, 0, 0)
            .unwrap(),
        duration_minutes: 60,
        title: Some(format!("Day {day}")),
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: None,
        notice_given_at: now.into(),
    }
}

/// A non-participant learns nothing: no session query runs at all.
#[tokio::test]
async fn a_non_participant_gets_nothing_and_no_session_is_queried() -> Result<(), Error> {
    let user_id = Id::new_v4();
    let relationship_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<coaching_relationships::Model, Vec<_>, _>(vec![vec![]])
        .into_connection();

    let sessions = find_nearest_by_user_with_includes(
        &db,
        user_id,
        window(relationship_id),
        IncludeOptions::default(),
    )
    .await?;

    assert!(sessions.is_empty());
    let log = db.into_transaction_log();
    assert_eq!(log.len(), 1, "only the participant check runs: {log:?}");
    assert!(log[0].statements()[0]
        .sql
        .contains(r#"FROM "refactor_platform"."coaching_relationships""#));

    Ok(())
}

/// Before is strict and latest first; after includes `around` and is earliest
/// first. Both scope to the relationship and tie-break on id.
#[tokio::test]
async fn a_participant_queries_each_side_of_around_with_its_own_limit() -> Result<(), Error> {
    let user_id = Id::new_v4();
    let relationship_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![relationship(relationship_id, user_id, Id::new_v4())]])
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![], vec![]])
        .into_connection();

    find_nearest_by_user_with_includes(
        &db,
        user_id,
        window(relationship_id),
        IncludeOptions::default(),
    )
    .await?;

    let log = db.into_transaction_log();
    assert_eq!(
        log[1..3],
        [
            Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    r#"SELECT {SESSION_COLUMNS} FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."coaching_relationship_id" = $1 AND "coaching_sessions"."date" < $2 ORDER BY "coaching_sessions"."date" DESC, "coaching_sessions"."id" DESC LIMIT $3"#
                ),
                [
                    relationship_id.into(),
                    around().into(),
                    sea_orm::Value::BigUnsigned(Some(2))
                ]
            ),
            Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    r#"SELECT {SESSION_COLUMNS} FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."coaching_relationship_id" = $1 AND "coaching_sessions"."date" >= $2 ORDER BY "coaching_sessions"."date" ASC, "coaching_sessions"."id" ASC LIMIT $3"#
                ),
                [
                    relationship_id.into(),
                    around().into(),
                    sea_orm::Value::BigUnsigned(Some(3))
                ]
            ),
        ]
    );

    Ok(())
}

/// The two sides merge latest first: `after` (queried earliest first) is reversed
/// ahead of `before` (already latest first).
#[tokio::test]
async fn the_two_sides_merge_latest_first() -> Result<(), Error> {
    let user_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let (b1, b2) = (session(relationship_id, 29), session(relationship_id, 28));
    let (a1, a2) = (session(relationship_id, 30), session(relationship_id, 31));

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![relationship(relationship_id, user_id, Id::new_v4())]])
        .append_query_results(vec![
            vec![b1.clone(), b2.clone()],
            vec![a1.clone(), a2.clone()],
        ])
        // View markers, first topic bodies, first goal titles.
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![], vec![], vec![]])
        .into_connection();

    let sessions = find_nearest_by_user_with_includes(
        &db,
        user_id,
        window(relationship_id),
        IncludeOptions::default(),
    )
    .await?;

    assert_eq!(
        sessions.iter().map(|s| s.session.id).collect::<Vec<_>>(),
        [a2.id, a1.id, b1.id, b2.id]
    );

    Ok(())
}

/// An invalid include combination is rejected before any query runs.
#[tokio::test]
async fn an_invalid_include_combination_runs_no_query() {
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let result = find_nearest_by_user_with_includes(
        &db,
        Id::new_v4(),
        window(Id::new_v4()),
        IncludeOptions {
            organization: true,
            ..Default::default()
        },
    )
    .await;

    assert!(result.is_err());
    assert!(db.into_transaction_log().is_empty());
}
