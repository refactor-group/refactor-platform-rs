use super::*;
use chrono::NaiveDate;
use entity::{coaching_session_topics, coaching_sessions_goals, goals};
use sea_orm::{DatabaseBackend, MockDatabase};

fn session(title: Option<&str>, date: NaiveDate) -> coaching_sessions::Model {
    let now = chrono::Utc::now();
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: Id::new_v4(),
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: date.and_hms_opt(23, 30, 0).unwrap(),
        duration_minutes: entity::duration::Duration::default_minutes(),
        title: title.map(str::to_owned),
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: None,
        notice_given_at: now.into(),
    }
}

/// Session rows, then the (empty) topic and goal tiers `batch_load_display_titles` reads.
fn db_with(sessions: Vec<coaching_sessions::Model>) -> DatabaseConnection {
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![sessions])
        .append_query_results(vec![Vec::<coaching_session_topics::Model>::new()])
        .append_query_results(vec![Vec::<(
            coaching_sessions_goals::Model,
            Option<goals::Model>,
        )>::new()])
        .into_connection()
}

#[tokio::test]
async fn a_titled_session_resolves_to_its_own_title() {
    let titled = session(
        Some("Quarterly review"),
        NaiveDate::from_ymd_opt(2026, 3, 9).unwrap(),
    );
    let db = db_with(vec![titled.clone()]);

    let titles = resolve_session_titles(&db, &[titled.id]).await.unwrap();

    assert_eq!(
        titles.get(&titled.id).map(String::as_str),
        Some("Quarterly review")
    );
}

#[tokio::test]
async fn an_untitled_session_falls_back_to_its_naive_date() {
    // 23:30 naive: the label must not shift a day under any timezone conversion.
    let untitled = session(None, NaiveDate::from_ymd_opt(2026, 3, 9).unwrap());
    let db = db_with(vec![untitled.clone()]);

    let titles = resolve_session_titles(&db, &[untitled.id]).await.unwrap();

    assert_eq!(
        titles.get(&untitled.id).map(String::as_str),
        Some("Coaching session — 2026-03-09")
    );
}

#[tokio::test]
async fn an_unknown_session_id_is_absent_from_the_map() {
    let db = db_with(vec![]);

    let titles = resolve_session_titles(&db, &[Id::new_v4()]).await.unwrap();

    assert!(titles.is_empty());
}

#[tokio::test]
async fn no_ids_issue_no_query() {
    // An empty mock fails any query, so success proves the short-circuit.
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let titles = resolve_session_titles(&db, &[]).await.unwrap();

    assert!(titles.is_empty());
}
