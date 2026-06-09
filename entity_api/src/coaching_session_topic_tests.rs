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
        carried_from_topic_id: None,
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
    // priority, status, and carried_from_topic_id ARE on the wire.
    assert!(value.get("priority").is_some());
    assert!(value.get("status").is_some());
    assert!(value.get("carried_from_topic_id").is_some());
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
            r#"SELECT "coaching_session_topics"."id", "coaching_session_topics"."coaching_session_id", "coaching_session_topics"."body", "coaching_session_topics"."user_id", "coaching_session_topics"."display_order", CAST("coaching_session_topics"."priority" AS "text"), CAST("coaching_session_topics"."status" AS "text"), "coaching_session_topics"."carried_from_topic_id", "coaching_session_topics"."created_at", "coaching_session_topics"."updated_at" FROM "refactor_platform"."coaching_session_topics" WHERE "coaching_session_topics"."coaching_session_id" = $1 ORDER BY "coaching_session_topics"."display_order" ASC, "coaching_session_topics"."created_at" ASC"#,
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

/// Collect every INSERT into coaching_session_topics from the transaction log,
/// returning each statement's bound values.
fn topic_insert_value_rows(log: &[Transaction]) -> Vec<Vec<Value>> {
    log.iter()
        .flat_map(|txn| txn.statements())
        .filter(|stmt| {
            stmt.sql.contains("INSERT INTO") && stmt.sql.contains(r#""coaching_session_topics""#)
        })
        .filter_map(|stmt| stmt.values.as_ref().map(|values| values.0.clone()))
        .collect()
}

/// Only Deferred topics carry: a source with [Open, Deferred, Discussed,
/// Deferred] yields exactly 2 copies. Two saved-copy results back exactly two
/// inserts; a stray insert of a non-Deferred topic would consume an absent
/// result and fail.
#[tokio::test]
async fn carry_over_copies_only_deferred_topics() {
    let source_id = Id::new_v4();
    let target_id = Id::new_v4();
    let open = topic_with(source_id, Id::new_v4(), 0, Status::Open, "open body");
    let deferred_a = topic_with(source_id, Id::new_v4(), 1, Status::Deferred, "deferred a");
    let discussed = topic_with(source_id, Id::new_v4(), 2, Status::Discussed, "disc body");
    let deferred_b = topic_with(source_id, Id::new_v4(), 3, Status::Deferred, "deferred b");

    let copy_a = topic_with(target_id, Id::new_v4(), 0, Status::Open, "deferred a");
    let copy_b = topic_with(target_id, Id::new_v4(), 1, Status::Open, "deferred b");

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // source SELECT (all 4), target SELECT (empty), then exactly 2 inserts.
        .append_query_results(vec![vec![
            open,
            deferred_a.clone(),
            discussed,
            deferred_b.clone(),
        ]])
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .append_query_results(vec![vec![copy_a]])
        .append_query_results(vec![vec![copy_b]])
        .into_connection();

    let carried = carry_over(&db, source_id, target_id).await.unwrap();
    assert_eq!(carried.len(), 2);

    // Copy shape: each INSERT binds status=open, carried_from_topic_id = the
    // matching Deferred source id (not Open/Discussed), preserves body and
    // priority, and appends display_order from the target's base (0, 1).
    let inserts = topic_insert_value_rows(&db.into_transaction_log());
    assert_eq!(inserts.len(), 2);
    for (row, (source, order)) in inserts
        .iter()
        .zip([(&deferred_a, 0_i32), (&deferred_b, 1_i32)])
    {
        assert!(
            row.contains(&Value::from("open")),
            "status should bind as open: {row:?}"
        );
        assert!(
            row.contains(&Value::from(source.id)),
            "carried_from_topic_id should be the Deferred source id: {row:?}"
        );
        assert!(
            row.contains(&Value::from(source.body.clone())),
            "body should be preserved: {row:?}"
        );
        assert!(
            row.contains(&Value::from("high")),
            "priority should be preserved: {row:?}"
        );
        assert!(
            row.contains(&Value::from(order)),
            "display_order should append from the target base: {row:?}"
        );
    }
}

/// The append base honors existing target topics: a target with one topic at
/// display_order 0 pushes the single carried copy to display_order 1.
#[tokio::test]
async fn carry_over_appends_after_existing_target_topics() {
    let source_id = Id::new_v4();
    let target_id = Id::new_v4();
    let deferred = topic_with(source_id, Id::new_v4(), 0, Status::Deferred, "deferred");
    let existing = topic_with(target_id, Id::new_v4(), 0, Status::Open, "existing");
    let copy = topic_with(target_id, Id::new_v4(), 1, Status::Open, "deferred");

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![deferred]])
        .append_query_results(vec![vec![existing]])
        .append_query_results(vec![vec![copy]])
        .into_connection();

    let carried = carry_over(&db, source_id, target_id).await.unwrap();
    assert_eq!(carried.len(), 1);

    let inserts = topic_insert_value_rows(&db.into_transaction_log());
    assert_eq!(inserts.len(), 1);
    assert!(
        inserts[0].contains(&Value::from(1_i32)),
        "carried copy should append at display_order 1: {:?}",
        inserts[0]
    );
}

/// A source with no Deferred topics carries nothing: empty Vec, no INSERT, only
/// the two SELECTs in the log.
#[tokio::test]
async fn carry_over_no_deferred_topics_returns_empty_and_inserts_nothing() {
    let source_id = Id::new_v4();
    let target_id = Id::new_v4();
    let open = topic_with(source_id, Id::new_v4(), 0, Status::Open, "open");
    let discussed = topic_with(source_id, Id::new_v4(), 1, Status::Discussed, "disc");

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![open, discussed]])
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .into_connection();

    let carried = carry_over(&db, source_id, target_id).await.unwrap();
    assert!(carried.is_empty());

    let log = db.into_transaction_log();
    assert!(
        topic_insert_value_rows(&log).is_empty(),
        "no INSERT should be emitted"
    );
    let selects = log
        .iter()
        .flat_map(|txn| txn.statements())
        .filter(|stmt| stmt.sql.contains("SELECT"))
        .count();
    assert_eq!(selects, 2, "only the source and target SELECTs run");
}

/// Dedupe on carried_from_topic_id: source has two Deferred topics d1, d2; the
/// target already holds a copy of d1. carry_over copies only d2. Exactly one
/// insert result is appended, so a stray copy of d1 would fail buffer-empty.
#[tokio::test]
async fn carry_over_skips_already_carried_source_topics() {
    let source_id = Id::new_v4();
    let target_id = Id::new_v4();
    let d1 = topic_with(source_id, Id::new_v4(), 0, Status::Deferred, "deferred 1");
    let d2 = topic_with(source_id, Id::new_v4(), 1, Status::Deferred, "deferred 2");

    // Target already contains a copy carried from d1.
    let mut existing_copy = topic_with(target_id, Id::new_v4(), 0, Status::Open, "deferred 1");
    existing_copy.carried_from_topic_id = Some(d1.id);

    let copy_d2 = topic_with(target_id, Id::new_v4(), 1, Status::Open, "deferred 2");

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // source SELECT (d1, d2), target SELECT (existing copy of d1), then ONE insert.
        .append_query_results(vec![vec![d1.clone(), d2.clone()]])
        .append_query_results(vec![vec![existing_copy]])
        .append_query_results(vec![vec![copy_d2]])
        .into_connection();

    let carried = carry_over(&db, source_id, target_id).await.unwrap();
    assert_eq!(carried.len(), 1, "only d2 is carried (d1 already present)");

    let inserts = topic_insert_value_rows(&db.into_transaction_log());
    assert_eq!(inserts.len(), 1);
    assert!(
        inserts[0].contains(&Value::from(d2.id)),
        "carried_from_topic_id should be d2: {:?}",
        inserts[0]
    );
    assert!(
        !inserts[0].contains(&Value::from(d1.id)),
        "must not re-carry d1: {:?}",
        inserts[0]
    );
}
