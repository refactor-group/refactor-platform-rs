use super::*;
use sea_orm::{DatabaseBackend, MockDatabase, Transaction, Value};

/// Builder for a topic Model with arbitrary body/timestamps.
fn topic(session_id: Id, id: Id, order: i32) -> Model {
    let now = chrono::Utc::now();
    Model {
        id,
        coaching_session_id: session_id,
        body: "topic body".to_owned(),
        user_id: Id::new_v4(),
        display_order: order,
        priority: Some(entity::topic_priority::Priority::High),
        status: entity::topic_status::Status::Open,
        moved_from_session_id: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// Topic with a chosen status, body, and priority (the fields carry-over reads).
fn topic_with(session_id: Id, id: Id, order: i32, status: Status, body: &str) -> Model {
    Model {
        status,
        body: body.to_owned(),
        ..topic(session_id, id, order)
    }
}

#[test]
fn next_display_order_empty_is_zero() {
    assert_eq!(next_display_order(&[]), 0);
}

#[test]
fn next_display_order_contiguous() {
    let session_id = Id::new_v4();
    let topics = [
        topic(session_id, Id::new_v4(), 0),
        topic(session_id, Id::new_v4(), 1),
        topic(session_id, Id::new_v4(), 2),
    ];
    assert_eq!(next_display_order(&topics), 3);
}

#[test]
fn next_display_order_with_gaps() {
    let session_id = Id::new_v4();
    let topics = [
        topic(session_id, Id::new_v4(), 0),
        topic(session_id, Id::new_v4(), 2),
        topic(session_id, Id::new_v4(), 5),
    ];
    assert_eq!(next_display_order(&topics), 6);
}

#[test]
fn reorder_request_is_valid_permutation() {
    let a = Id::new_v4();
    let b = Id::new_v4();
    let c = Id::new_v4();
    assert!(reorder_request_is_valid(&[a, b, c], &[c, a, b]));
}

#[test]
fn reorder_request_is_valid_missing_id() {
    let a = Id::new_v4();
    let b = Id::new_v4();
    let c = Id::new_v4();
    assert!(!reorder_request_is_valid(&[a, b, c], &[a, b]));
}

#[test]
fn reorder_request_is_valid_unknown_id() {
    let a = Id::new_v4();
    let b = Id::new_v4();
    let c = Id::new_v4();
    let d = Id::new_v4();
    assert!(!reorder_request_is_valid(&[a, b, c], &[a, b, d]));
}

#[test]
fn reorder_request_is_valid_duplicate_id() {
    let a = Id::new_v4();
    let b = Id::new_v4();
    assert!(!reorder_request_is_valid(&[a, b], &[a, a]));
}

#[test]
fn display_order_is_never_serialized() {
    let value = serde_json::to_value(topic(Id::new_v4(), Id::new_v4(), 7)).unwrap();
    assert!(value.get("display_order").is_none());
    assert!(value.get("body").is_some());
    assert!(value.get("id").is_some());
    assert!(value.get("coaching_session_id").is_some());
    assert!(value.get("user_id").is_some());
    // priority, status, and moved_from_session_id ARE on the wire.
    assert!(value.get("priority").is_some());
    assert!(value.get("status").is_some());
    assert!(value.get("moved_from_session_id").is_some());
}

#[tokio::test]
async fn find_by_coaching_session_id_orders_by_display_order_then_created_at() {
    let session_id = Id::new_v4();
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .into_connection();

    let _ = find_by_coaching_session_id(&db, session_id).await;

    assert_eq!(
        db.into_transaction_log(),
        [Transaction::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT "coaching_session_topics"."id", "coaching_session_topics"."coaching_session_id", "coaching_session_topics"."body", "coaching_session_topics"."user_id", "coaching_session_topics"."display_order", CAST("coaching_session_topics"."priority" AS "text"), CAST("coaching_session_topics"."status" AS "text"), "coaching_session_topics"."moved_from_session_id", "coaching_session_topics"."created_at", "coaching_session_topics"."updated_at" FROM "refactor_platform"."coaching_session_topics" WHERE "coaching_session_topics"."coaching_session_id" = $1 ORDER BY "coaching_session_topics"."display_order" ASC, "coaching_session_topics"."created_at" ASC"#,
            [session_id.into()]
        )]
    );
}

#[tokio::test]
async fn reorder_rejects_non_permutation_id_set() {
    let session_id = Id::new_v4();
    let a = topic(session_id, Id::new_v4(), 0);
    let b = topic(session_id, Id::new_v4(), 1);
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![a.clone(), b.clone()]])
        .into_connection();

    let result = reorder(&db, session_id, vec![a.id, Id::new_v4()]).await;

    assert!(matches!(
        result,
        Err(Error {
            error_kind: EntityApiErrorKind::TopicReorderMismatch,
            ..
        })
    ));
}

/// Collect every UPDATE of coaching_session_topics from the transaction log,
/// returning each statement's bound values.
fn topic_update_value_rows(log: &[Transaction]) -> Vec<Vec<Value>> {
    log.iter()
        .flat_map(|txn| txn.statements())
        .filter(|stmt| {
            stmt.sql.contains("UPDATE") && stmt.sql.contains(r#""coaching_session_topics""#)
        })
        .filter_map(|stmt| stmt.values.as_ref().map(|values| values.0.clone()))
        .collect()
}

/// move_topic re-parents one topic: the UPDATE binds coaching_session_id = target,
/// status = open, moved_from_session_id = source, and a fresh display_order from the
/// target's base (here 1, the target already holds one topic at order 0).
#[tokio::test]
async fn move_topic_reparents_and_resets() {
    let source_id = Id::new_v4();
    let target_id = Id::new_v4();
    let deferred = topic_with(source_id, Id::new_v4(), 0, Status::Deferred, "deferred");
    let existing = topic_with(target_id, Id::new_v4(), 0, Status::Open, "existing");
    let moved = topic_with(target_id, deferred.id, 1, Status::Open, "deferred");

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // target-topics fetch (for base) → topic find_by_id → UPDATE.
        .append_query_results(vec![vec![existing]])
        .append_query_results(vec![vec![deferred.clone()]])
        .append_query_results(vec![vec![moved]])
        .into_connection();

    let result = move_topic(&db, deferred.id, target_id, Some(source_id))
        .await
        .unwrap();
    assert_eq!(result.coaching_session_id, target_id);

    let updates = topic_update_value_rows(&db.into_transaction_log());
    assert_eq!(updates.len(), 1);
    assert!(
        updates[0].contains(&Value::from(target_id)),
        "coaching_session_id should be the target: {:?}",
        updates[0]
    );
    assert!(
        updates[0].contains(&Value::from("open")),
        "status should bind as open: {:?}",
        updates[0]
    );
    assert!(
        updates[0].contains(&Value::from(source_id)),
        "moved_from_session_id should be the source: {:?}",
        updates[0]
    );
    assert!(
        updates[0].contains(&Value::from(1_i32)),
        "display_order should append from the target base: {:?}",
        updates[0]
    );
}

/// move_deferred_to_session moves only Deferred topics: a source with
/// [Open, Deferred, Discussed] into an empty target yields exactly ONE UPDATE,
/// re-parenting the Deferred topic to the target with status open + moved_from =
/// source. One moved-row result backs exactly one UPDATE; a stray move of a
/// non-Deferred topic would consume an absent result and fail.
#[tokio::test]
async fn move_deferred_to_session_moves_only_deferred() {
    let source_id = Id::new_v4();
    let target_id = Id::new_v4();
    let open = topic_with(source_id, Id::new_v4(), 0, Status::Open, "open body");
    let deferred = topic_with(source_id, Id::new_v4(), 1, Status::Deferred, "deferred");
    let discussed = topic_with(source_id, Id::new_v4(), 2, Status::Discussed, "disc body");
    let moved_row = topic_with(target_id, deferred.id, 0, Status::Open, "deferred");

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // source SELECT (all 3) → target SELECT (empty, for base) → ONE UPDATE.
        .append_query_results(vec![vec![open, deferred.clone(), discussed]])
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .append_query_results(vec![vec![moved_row]])
        .into_connection();

    let moved = move_deferred_to_session(&db, source_id, target_id)
        .await
        .unwrap();
    assert_eq!(moved.len(), 1);

    let updates = topic_update_value_rows(&db.into_transaction_log());
    assert_eq!(updates.len(), 1);
    assert!(
        updates[0].contains(&Value::from(target_id)),
        "coaching_session_id should be the target: {:?}",
        updates[0]
    );
    assert!(
        updates[0].contains(&Value::from("open")),
        "status should bind as open: {:?}",
        updates[0]
    );
    assert!(
        updates[0].contains(&Value::from(source_id)),
        "moved_from_session_id should be the source: {:?}",
        updates[0]
    );
}
