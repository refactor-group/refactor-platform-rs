use super::*;
use crate::coaching_relationships;
use crate::coaching_sessions;
use crate::events::DomainEvent;
use crate::test_support::recording_publisher;
use entity::Id;
use sea_orm::{DatabaseBackend, MockDatabase};

fn topic_model(coaching_session_id: Id) -> Model {
    let now = chrono::Utc::now().fixed_offset();
    Model {
        id: Id::new_v4(),
        coaching_session_id,
        body: "Topic body".to_string(),
        user_id: Id::new_v4(),
        display_order: 0,
        priority: Some(Priority::High),
        status: Status::Open,
        carried_from_topic_id: None,
        created_at: now,
        updated_at: now,
    }
}

fn session_with_relationship(
    coaching_session_id: Id,
) -> (coaching_sessions::Model, coaching_relationships::Model) {
    let now = chrono::Utc::now().fixed_offset();
    let relationship_id = Id::new_v4();
    let session = coaching_sessions::Model {
        id: coaching_session_id,
        coaching_relationship_id: relationship_id,
        collab_document_name: None,
        date: now.naive_utc(),
        duration_minutes: 60,
        meeting_url: None,
        provider: None,
        created_at: now,
        updated_at: now,
        hydrated_at: None,
    };
    let relationship = coaching_relationships::Model {
        id: relationship_id,
        organization_id: Id::new_v4(),
        coach_id: Id::new_v4(),
        coachee_id: Id::new_v4(),
        slug: "test-slug".to_string(),
        created_at: now,
        updated_at: now,
    };
    (session, relationship)
}

/// A bare coaching_sessions::Model on `relationship_id` at `date`.
fn coaching_session(
    coaching_session_id: Id,
    relationship_id: Id,
    date: chrono::NaiveDateTime,
) -> coaching_sessions::Model {
    let now = chrono::Utc::now().fixed_offset();
    coaching_sessions::Model {
        id: coaching_session_id,
        coaching_relationship_id: relationship_id,
        collab_document_name: None,
        date,
        duration_minutes: 60,
        meeting_url: None,
        provider: None,
        created_at: now,
        updated_at: now,
        hydrated_at: None,
    }
}

/// Asserts the recorder captured exactly one TopicsChanged for the session.
fn assert_topics_changed(events: &[DomainEvent], expected_session_id: Id) {
    assert_eq!(events.len(), 1, "expected exactly one published event");
    match &events[0] {
        DomainEvent::TopicsChanged {
            coaching_session_id,
            ..
        } => assert_eq!(*coaching_session_id, expected_session_id),
        other => panic!("expected TopicsChanged, got {other:?}"),
    }
}

#[tokio::test]
async fn create_publishes_topics_changed() {
    let session_id = Id::new_v4();
    let created = topic_model(session_id);
    let (publisher, events) = recording_publisher();

    // create: existing-topics load (all) → insert (save) → participant lookup (find_also_related)
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![Vec::<Model>::new()])
        .append_query_results(vec![vec![created.clone()]])
        .append_query_results(vec![vec![session_with_relationship(session_id)]])
        .into_connection();

    let result = create(
        &db,
        &publisher,
        session_id,
        "Topic body".to_string(),
        Id::new_v4(),
        None,
    )
    .await;

    assert!(result.is_ok());
    assert_topics_changed(&events.lock().unwrap(), session_id);
}

#[tokio::test]
async fn reorder_publishes_topics_changed() {
    let session_id = Id::new_v4();
    let topic_a = topic_model(session_id);
    let topic_b = topic_model(session_id);
    let ordered = vec![topic_b.id, topic_a.id];
    let (publisher, events) = recording_publisher();

    // reorder: load current → per-id update (x2) → reload → participant lookup
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![topic_a.clone(), topic_b.clone()]])
        .append_query_results(vec![vec![topic_b.clone()]])
        .append_query_results(vec![vec![topic_a.clone()]])
        .append_query_results(vec![vec![topic_b.clone(), topic_a.clone()]])
        .append_query_results(vec![vec![session_with_relationship(session_id)]])
        .into_connection();

    let result = reorder(&db, &publisher, session_id, ordered).await;

    assert!(result.is_ok());
    assert_topics_changed(&events.lock().unwrap(), session_id);
}

#[tokio::test]
async fn set_status_publishes_topics_changed() {
    let session_id = Id::new_v4();
    let topic = topic_model(session_id);
    let (publisher, events) = recording_publisher();

    // set_status: load topic (find_by_id) → update → participant lookup
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![topic.clone()]])
        .append_query_results(vec![vec![topic.clone()]])
        .append_query_results(vec![vec![session_with_relationship(session_id)]])
        .into_connection();

    let result = set_status(&db, &publisher, topic.id, Status::Discussed).await;

    assert!(result.is_ok());
    assert_topics_changed(&events.lock().unwrap(), session_id);
}

/// Late defer: setting a topic to Deferred when the next session already exists
/// carries it forward immediately and publishes TopicsChanged for BOTH the
/// source session (first) and the next session (second).
#[tokio::test]
async fn set_status_deferred_carries_over_to_existing_next_session() {
    let source_session_id = Id::new_v4();
    let next_session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let source_date = chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap();
    let next_date = chrono::NaiveDate::from_ymd_opt(2026, 6, 8)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap();

    let topic = topic_model(source_session_id);
    let deferred = Model {
        status: Status::Deferred,
        ..topic.clone()
    };
    let source_session = coaching_session(source_session_id, relationship_id, source_date);
    let next_session = coaching_session(next_session_id, relationship_id, next_date);
    let carried_copy = Model {
        id: Id::new_v4(),
        coaching_session_id: next_session_id,
        status: Status::Open,
        carried_from_topic_id: Some(deferred.id),
        ..topic.clone()
    };

    let (publisher, events) = recording_publisher();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // set_status: find_by_id(topic) → update (now Deferred).
        .append_query_results(vec![vec![topic.clone()]])
        .append_query_results(vec![vec![deferred.clone()]])
        // find_by_id(source session).
        .append_query_results(vec![vec![source_session.clone()]])
        // find_next_session → next session.
        .append_query_results(vec![vec![next_session.clone()]])
        // carry_over: source-topics fetch → target-topics fetch (empty) → insert.
        .append_query_results(vec![vec![deferred.clone()]])
        .append_query_results(vec![Vec::<Model>::new()])
        .append_query_results(vec![vec![carried_copy.clone()]])
        // participant lookup for source, then for next.
        .append_query_results(vec![vec![session_with_relationship(source_session_id)]])
        .append_query_results(vec![vec![session_with_relationship(next_session_id)]])
        .into_connection();

    let result = set_status(&db, &publisher, topic.id, Status::Deferred).await;
    assert!(result.is_ok());

    let recorded = events.lock().unwrap();
    let topics_changed: Vec<Id> = recorded
        .iter()
        .filter_map(|e| match e {
            DomainEvent::TopicsChanged {
                coaching_session_id,
                ..
            } => Some(*coaching_session_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        topics_changed,
        vec![source_session_id, next_session_id],
        "expected TopicsChanged for source then next session"
    );
}
