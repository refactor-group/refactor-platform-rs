use super::*;
use crate::coaching_relationships;
use crate::coaching_sessions;
use crate::events::DomainEvent;
use crate::test_support::recording_publisher;
use entity::coaching_session_topics::TopicSnapshot;
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
        moved_from_session_id: None,
        undo_snapshot: None,
        deleted_at: None,
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
        coaching_session_series_id: None,
        collab_document_name: None,
        date: now.naive_utc(),
        duration_minutes: 60,
        title: None,
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
        coaching_session_series_id: None,
        collab_document_name: None,
        date,
        duration_minutes: 60,
        title: None,
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
/// MOVES it forward (re-parents) immediately. The returned topic lives at the next
/// session with status Open and moved_from = source, and TopicsChanged publishes for
/// the destination (next) FIRST, then the origin (source).
#[tokio::test]
async fn set_status_deferred_moves_to_existing_next_session() {
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
    let source_session = coaching_session(source_session_id, relationship_id, source_date);
    let next_session = coaching_session(next_session_id, relationship_id, next_date);
    let moved = Model {
        coaching_session_id: next_session_id,
        status: Status::Open,
        moved_from_session_id: Some(source_session_id),
        undo_snapshot: Some(TopicSnapshot {
            coaching_session_id: source_session_id,
            body: topic.body.clone(),
            display_order: topic.display_order,
            priority: topic.priority.clone(),
            status: topic.status.clone(),
            moved_from_session_id: topic.moved_from_session_id,
            deleted_at: None,
            updated_at: topic.updated_at,
        }),
        ..topic.clone()
    };

    let (publisher, events) = recording_publisher();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // set_status defer path: find_by_id(topic) → find_by_id(source session) →
        // find_next_session → next session.
        .append_query_results(vec![vec![topic.clone()]])
        .append_query_results(vec![vec![source_session.clone()]])
        .append_query_results(vec![vec![next_session.clone()]])
        // defer_move: target-topics fetch (empty) → topic find_by_id → UPDATE.
        .append_query_results(vec![Vec::<Model>::new()])
        .append_query_results(vec![vec![topic.clone()]])
        .append_query_results(vec![vec![moved.clone()]])
        // participant lookup for dest (next), then for origin (source).
        .append_query_results(vec![vec![session_with_relationship(next_session_id)]])
        .append_query_results(vec![vec![session_with_relationship(source_session_id)]])
        .into_connection();

    let result = set_status(&db, &publisher, topic.id, Status::Deferred)
        .await
        .unwrap();
    assert_eq!(result.coaching_session_id, next_session_id);
    assert_eq!(result.status, Status::Open);
    assert_eq!(result.moved_from_session_id, Some(source_session_id));
    assert!(
        result.undo_snapshot.is_some(),
        "defer snapshots pre-defer state"
    );

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
        vec![next_session_id, source_session_id],
        "expected TopicsChanged for next (dest) then source (origin)"
    );
}

/// Defer with no next session HOLDS: the topic persists as Deferred in place (the
/// hydration hook moves it later), and exactly one TopicsChanged fires in place.
#[tokio::test]
async fn set_status_deferred_with_no_next_session_holds() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap();

    let topic = topic_model(session_id);
    let deferred = Model {
        status: Status::Deferred,
        undo_snapshot: Some(TopicSnapshot {
            coaching_session_id: session_id,
            body: topic.body.clone(),
            display_order: topic.display_order,
            priority: topic.priority.clone(),
            status: topic.status.clone(),
            moved_from_session_id: topic.moved_from_session_id,
            deleted_at: None,
            updated_at: topic.updated_at,
        }),
        ..topic.clone()
    };
    let session = coaching_session(session_id, relationship_id, date);

    let (publisher, events) = recording_publisher();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // defer path: find_by_id(topic) → find_by_id(session) → find_next_session (none).
        .append_query_results(vec![vec![topic.clone()]])
        .append_query_results(vec![vec![session.clone()]])
        .append_query_results(vec![Vec::<coaching_sessions::Model>::new()])
        // defer_hold: find_by_id(topic) → UPDATE (now Deferred, snapshot set).
        .append_query_results(vec![vec![topic.clone()]])
        .append_query_results(vec![vec![deferred.clone()]])
        // participant lookup (in place).
        .append_query_results(vec![vec![session_with_relationship(session_id)]])
        .into_connection();

    let result = set_status(&db, &publisher, topic.id, Status::Deferred)
        .await
        .unwrap();
    assert_eq!(result.coaching_session_id, session_id);
    assert_eq!(result.status, Status::Deferred);
    assert!(
        result.undo_snapshot.is_some(),
        "hold snapshots pre-defer state"
    );

    assert_topics_changed(&events.lock().unwrap(), session_id);
}

/// Re-deferring an already-held Deferred topic (still no next session) is a no-op: it must NOT
/// overwrite the original pre-defer snapshot, so undo can still reach the Open state. Guards a
/// snapshot-clobber that would otherwise strand the topic Deferred with no undo path back.
#[tokio::test]
async fn set_status_deferred_again_preserves_original_snapshot() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap();

    let topic = topic_model(session_id);
    // The original pre-defer snapshot, captured when the topic was Open.
    let original_snapshot = TopicSnapshot {
        coaching_session_id: session_id,
        body: topic.body.clone(),
        display_order: topic.display_order,
        priority: topic.priority.clone(),
        status: Status::Open,
        moved_from_session_id: None,
        deleted_at: None,
        updated_at: topic.updated_at,
    };
    let held = Model {
        status: Status::Deferred,
        undo_snapshot: Some(original_snapshot.clone()),
        ..topic.clone()
    };
    let session = coaching_session(session_id, relationship_id, date);

    let (publisher, events) = recording_publisher();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // defer path: find_by_id(topic=held) → find_by_id(session) → find_next_session (none).
        .append_query_results(vec![vec![held.clone()]])
        .append_query_results(vec![vec![session.clone()]])
        .append_query_results(vec![Vec::<coaching_sessions::Model>::new()])
        // guard short-circuits before any defer_hold UPDATE; only the participant lookup remains.
        .append_query_results(vec![vec![session_with_relationship(session_id)]])
        .into_connection();

    let result = set_status(&db, &publisher, held.id, Status::Deferred)
        .await
        .unwrap();

    assert_eq!(result.status, Status::Deferred, "held topic stays Deferred");
    assert_eq!(
        result.undo_snapshot,
        Some(original_snapshot),
        "re-defer must preserve the original pre-defer snapshot, not overwrite it"
    );

    // Teeth: the no-op must issue no topic UPDATE (an UPDATE would clobber the snapshot).
    let wrote_topic = db
        .into_transaction_log()
        .iter()
        .flat_map(|txn| txn.statements())
        .any(|stmt| {
            stmt.sql.contains("UPDATE") && stmt.sql.contains(r#""coaching_session_topics""#)
        });
    assert!(
        !wrote_topic,
        "re-defer of a held topic must not write the topic row"
    );

    assert_topics_changed(&events.lock().unwrap(), session_id);
}

/// Undo a moved topic restores the PRE-defer status (not Open) at the origin and publishes
/// TopicsChanged for the restored session (origin) first, then the old current session.
#[tokio::test]
async fn undo_restores_pre_defer_status() {
    let origin_session_id = Id::new_v4();
    let current_session_id = Id::new_v4();
    let t0 = chrono::Utc::now().fixed_offset();

    let moved = Model {
        coaching_session_id: current_session_id,
        status: Status::Open,
        moved_from_session_id: Some(origin_session_id),
        undo_snapshot: Some(TopicSnapshot {
            coaching_session_id: origin_session_id,
            body: "Topic body".to_string(),
            display_order: 4,
            priority: Some(Priority::High),
            status: Status::Discussed,
            moved_from_session_id: None,
            deleted_at: None,
            updated_at: t0,
        }),
        ..topic_model(current_session_id)
    };
    let restored = Model {
        coaching_session_id: origin_session_id,
        status: Status::Discussed,
        display_order: 4,
        moved_from_session_id: None,
        updated_at: t0,
        undo_snapshot: None,
        ..moved.clone()
    };

    let (publisher, events) = recording_publisher();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // undo: find_including_deleted_by_id(before) → restore_from_snapshot [find_including_deleted_by_id → UPDATE].
        .append_query_results(vec![vec![moved.clone()]])
        .append_query_results(vec![vec![moved.clone()]])
        .append_query_results(vec![vec![restored.clone()]])
        // participant lookup for restored (origin), then old current (notify_other).
        .append_query_results(vec![vec![session_with_relationship(origin_session_id)]])
        .append_query_results(vec![vec![session_with_relationship(current_session_id)]])
        .into_connection();

    let result = undo(&db, &publisher, moved.id).await.unwrap();
    assert_eq!(result.coaching_session_id, origin_session_id);
    assert_eq!(result.status, Status::Discussed);
    assert_eq!(result.moved_from_session_id, None);
    assert_eq!(result.undo_snapshot, None);

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
        vec![origin_session_id, current_session_id],
        "expected TopicsChanged for restored origin then old current"
    );
}

/// Undo a topic without a snapshot (nothing to undo) is a validation error (no publish).
#[tokio::test]
async fn undo_without_snapshot_is_validation_error() {
    let session_id = Id::new_v4();
    let settled = topic_model(session_id); // undo_snapshot: None

    let (publisher, events) = recording_publisher();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // undo: find_including_deleted_by_id(before) → restore_from_snapshot find_including_deleted_by_id (no snapshot → None).
        .append_query_results(vec![vec![settled.clone()]])
        .append_query_results(vec![vec![settled.clone()]])
        .into_connection();

    let err = undo(&db, &publisher, settled.id).await.unwrap_err();
    assert!(matches!(err.error_kind, DomainErrorKind::Validation(_)));
    assert!(events.lock().unwrap().is_empty());
}

/// Undo a soft-deleted topic un-deletes it in place: deleted_at returns to NULL and the
/// snapshot's status is restored. One affected session (old == new), so one publish.
#[tokio::test]
async fn undo_restores_a_soft_deleted_topic() {
    let session_id = Id::new_v4();
    let t0 = chrono::Utc::now().fixed_offset();

    let deleted = Model {
        deleted_at: Some(t0),
        undo_snapshot: Some(TopicSnapshot {
            coaching_session_id: session_id,
            body: "Topic body".to_string(),
            display_order: 0,
            priority: Some(Priority::High),
            status: Status::Discussed,
            moved_from_session_id: None,
            deleted_at: None,
            updated_at: t0,
        }),
        ..topic_model(session_id)
    };
    let restored = Model {
        status: Status::Discussed,
        deleted_at: None,
        undo_snapshot: None,
        ..deleted.clone()
    };

    let (publisher, events) = recording_publisher();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // undo: find_including_deleted_by_id(before) → restore_from_snapshot [find_including_deleted_by_id → UPDATE].
        .append_query_results(vec![vec![deleted.clone()]])
        .append_query_results(vec![vec![deleted.clone()]])
        .append_query_results(vec![vec![restored.clone()]])
        // participant lookup (one session; old == new).
        .append_query_results(vec![vec![session_with_relationship(session_id)]])
        .into_connection();

    let result = undo(&db, &publisher, deleted.id).await.unwrap();
    assert_eq!(result.status, Status::Discussed);
    assert_eq!(result.deleted_at, None);
    assert_eq!(result.undo_snapshot, None);
    assert_topics_changed(&events.lock().unwrap(), session_id);
}
