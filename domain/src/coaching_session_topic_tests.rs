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
