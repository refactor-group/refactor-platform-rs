//! The statement `find_nearest_by_user_with_includes` sends and its control flow; which
//! rows the statement selects is covered by the SQLite tier in `domain`.

use chrono::{NaiveDate, Utc};
use entity::Id;
use sea_orm::{DatabaseBackend, MockDatabase};

use super::*;

/// A non-SuperAdmin `user_id` holding a role in one organization.
fn viewer(user_id: Id) -> Visibility {
    Visibility {
        user_id,
        is_super_admin: false,
        admin_org_ids: vec![],
        member_org_ids: vec![Id::new_v4()],
    }
}

const SESSION_COLUMNS: &str = r#""coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."coaching_session_series_id", "coaching_sessions"."ical_sequence", "coaching_sessions"."ical_recurrence_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."title", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at", "coaching_sessions"."notice_given_at""#;

fn around() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 30)
        .unwrap()
        .and_hms_opt(14, 0, 0)
        .unwrap()
}

fn window(coaching_relationship_ids: &[Id]) -> NearestWindow {
    NearestWindow {
        coaching_relationship_ids: coaching_relationship_ids.to_vec(),
        around: around(),
        before: 2,
        after: 3,
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

/// One statement reads every relationship's window: for each relationship, the latest
/// sessions before `around` and the earliest at or after it, each part an ordered read that
/// stops at its count. Only relationships the user is part of count, latest first.
#[tokio::test]
async fn one_statement_reads_every_relationships_window() -> Result<(), Error> {
    let user_id = Id::new_v4();
    let (first, second) = (Id::new_v4(), Id::new_v4());

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .into_connection();

    find_nearest_by_user_with_includes(
        &db,
        &viewer(user_id),
        window(&[first, second]),
        IncludeOptions::default(),
    )
    .await?;

    let log = db.into_transaction_log();
    let statements: Vec<_> = log.iter().flat_map(|txn| txn.statements()).collect();
    assert_eq!(statements.len(), 1, "one statement: {statements:?}");
    let sql = &statements[0].sql;

    let (outer, ids) = sql
        .split_once(r#"AND "coaching_sessions"."id" IN ("#)
        .expect("sessions are read by id");
    assert!(outer.starts_with(&format!("SELECT {SESSION_COLUMNS} FROM ")));
    assert!(outer.contains(r#"WHERE ("coaching_relationships"."coach_id" = "#));
    assert!(outer.contains(r#"OR "coaching_relationships"."coachee_id" = "#));
    assert!(ids
        .ends_with(r#") ORDER BY "coaching_sessions"."date" DESC, "coaching_sessions"."id" DESC"#));

    let parts: Vec<&str> = ids.split("UNION ALL").collect();
    assert_eq!(
        parts.len(),
        4,
        "a before and an after part per relationship: {ids}"
    );
    for (part, (date, order)) in parts.iter().zip([
        (r#""date" < "#, "DESC"),
        (r#""date" >= "#, "ASC"),
        (r#""date" < "#, "DESC"),
        (r#""date" >= "#, "ASC"),
    ]) {
        assert!(
            part.contains(r#"WHERE "coaching_sessions"."coaching_relationship_id" = "#),
            "{part}"
        );
        assert!(part.contains(date), "{part}");
        assert!(part.contains(&format!(
            r#"ORDER BY "coaching_sessions"."date" {order}, "coaching_sessions"."id" {order} LIMIT "#
        )), "{part}");
    }

    let values = statements[0]
        .values
        .as_ref()
        .expect("bound values")
        .0
        .clone();
    for expected in [
        first.into(),
        second.into(),
        user_id.into(),
        around().into(),
        sea_orm::Value::BigUnsigned(Some(2)),
        sea_orm::Value::BigUnsigned(Some(3)),
    ] {
        assert!(values.contains(&expected), "{expected:?} in {values:?}");
    }

    Ok(())
}

/// A window naming no relationships reads nothing.
#[tokio::test]
async fn a_window_with_no_relationships_runs_no_query() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let sessions = find_nearest_by_user_with_includes(
        &db,
        &viewer(Id::new_v4()),
        window(&[]),
        IncludeOptions::default(),
    )
    .await?;

    assert!(sessions.is_empty());
    assert!(db.into_transaction_log().is_empty());
    Ok(())
}

/// The statement's order is the response's order; nothing reorders the rows after.
#[tokio::test]
async fn sessions_keep_the_statements_order() -> Result<(), Error> {
    let relationship_id = Id::new_v4();
    let rows = vec![
        session(relationship_id, 31),
        session(relationship_id, 30),
        session(relationship_id, 29),
    ];

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![rows.clone()])
        // View markers, first topic bodies, first goal titles.
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![], vec![], vec![]])
        .into_connection();

    let sessions = find_nearest_by_user_with_includes(
        &db,
        &viewer(Id::new_v4()),
        window(&[relationship_id]),
        IncludeOptions::default(),
    )
    .await?;

    assert_eq!(
        sessions.iter().map(|s| s.session.id).collect::<Vec<_>>(),
        rows.iter().map(|s| s.id).collect::<Vec<_>>()
    );

    Ok(())
}

/// An invalid include combination is rejected before any query runs.
#[tokio::test]
async fn an_invalid_include_combination_runs_no_query() {
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let result = find_nearest_by_user_with_includes(
        &db,
        &viewer(Id::new_v4()),
        window(&[Id::new_v4()]),
        IncludeOptions {
            organization: true,
            ..Default::default()
        },
    )
    .await;

    assert!(result.is_err());
    assert!(db.into_transaction_log().is_empty());
}
