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
        undo_snapshot: None,
        deleted_at: None,
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
    // undo_snapshot and deleted_at are server-only (#[serde(skip)]).
    assert!(value.get("undo_snapshot").is_none());
    assert!(value.get("deleted_at").is_none());
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
            r#"SELECT "coaching_session_topics"."id", "coaching_session_topics"."coaching_session_id", "coaching_session_topics"."body", "coaching_session_topics"."user_id", "coaching_session_topics"."display_order", CAST("coaching_session_topics"."priority" AS "text"), CAST("coaching_session_topics"."status" AS "text"), "coaching_session_topics"."moved_from_session_id", "coaching_session_topics"."undo_snapshot", "coaching_session_topics"."deleted_at", "coaching_session_topics"."created_at", "coaching_session_topics"."updated_at" FROM "refactor_platform"."coaching_session_topics" WHERE "coaching_session_topics"."coaching_session_id" = $1 AND "coaching_session_topics"."deleted_at" IS NULL ORDER BY "coaching_session_topics"."display_order" ASC, "coaching_session_topics"."created_at" ASC"#,
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

/// defer_move re-parents one topic and snapshots its PRE-defer state: the returned topic
/// lives at the target with status Open and moved_from = origin, while undo_snapshot
/// captures the row exactly as it was (origin session, Discussed, original order/moved_from/updated_at).
#[tokio::test]
async fn defer_move_snapshots_and_reparents() {
    let origin_id = Id::new_v4();
    let target_id = Id::new_v4();
    let original = topic_with(origin_id, Id::new_v4(), 2, Status::Discussed, "deferred");
    let existing = topic_with(target_id, Id::new_v4(), 0, Status::Open, "existing");
    // The DB returns the post-update row carrying the snapshot of the pre-defer state.
    let moved = Model {
        coaching_session_id: target_id,
        status: Status::Open,
        moved_from_session_id: Some(origin_id),
        display_order: 1,
        undo_snapshot: Some(TopicSnapshot {
            coaching_session_id: origin_id,
            body: original.body.clone(),
            display_order: original.display_order,
            priority: original.priority.clone(),
            status: Status::Discussed,
            moved_from_session_id: original.moved_from_session_id,
            deleted_at: None,
            updated_at: original.updated_at,
        }),
        ..original.clone()
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // target-topics fetch (for base) → topic find_by_id → UPDATE.
        .append_query_results(vec![vec![existing]])
        .append_query_results(vec![vec![original.clone()]])
        .append_query_results(vec![vec![moved]])
        .into_connection();

    let result = defer_move(&db, original.id, target_id).await.unwrap();
    assert_eq!(result.coaching_session_id, target_id);
    assert_eq!(result.status, Status::Open);
    assert_eq!(result.moved_from_session_id, Some(origin_id));
    // The snapshot is the PRE-defer state, not the moved state.
    assert_eq!(
        result.undo_snapshot,
        Some(TopicSnapshot {
            coaching_session_id: origin_id,
            body: original.body.clone(),
            display_order: original.display_order,
            priority: original.priority.clone(),
            status: Status::Discussed,
            moved_from_session_id: original.moved_from_session_id,
            deleted_at: None,
            updated_at: original.updated_at,
        })
    );

    // The return-value asserts above only check the canned Mock row; verify the actual UPDATE
    // re-parents (coaching_session_id=target, display_order=1 appended, status=open,
    // moved_from=origin) AND binds undo_snapshot = the captured pre-defer state. deleted_at is
    // left Unchanged (not in the SET clause), so the SET-bind count is unchanged. The two
    // updated_at timestamps (column + snapshot) are runtime now(), so we assert their fields
    // structurally rather than pinning the exact instant.
    let updates = topic_update_value_rows(&db.into_transaction_log());
    assert_eq!(updates.len(), 1);
    let row = &updates[0];
    assert_eq!(row.len(), 7);
    assert_eq!(row[0], Value::from(target_id)); // coaching_session_id -> target
    assert_eq!(row[1], Value::from(1_i32)); // display_order -> appended from base
    assert_eq!(row[2], Value::from("open")); // status -> open
    assert_eq!(row[3], Value::from(origin_id)); // moved_from_session_id -> origin
    assert_eq!(row[6], Value::from(original.id)); // WHERE id

    // undo_snapshot (JSONB) captures the PRE-defer state, not the re-parented state.
    let Value::Json(Some(snapshot_json)) = &row[4] else {
        panic!("undo_snapshot should bind a JSON object: {:?}", row[4]);
    };
    assert_eq!(snapshot_json["coaching_session_id"], origin_id.to_string());
    assert_eq!(snapshot_json["status"], "Discussed");
    assert_eq!(snapshot_json["display_order"], original.display_order);
    assert_eq!(
        snapshot_json["moved_from_session_id"],
        serde_json::Value::Null
    );
    assert_eq!(snapshot_json["deleted_at"], serde_json::Value::Null);
    // Compare as instants: clock sub-microsecond precision is platform-dependent.
    let snapshot_updated_at: chrono::DateTime<chrono::FixedOffset> =
        serde_json::from_value(snapshot_json["updated_at"].clone()).unwrap();
    assert_eq!(snapshot_updated_at, original.updated_at);
}

/// restore_from_snapshot writes the FULL captured prior row back (location, status, position,
/// moved_from, deleted_at, updated_at) and clears the buffer in the same write.
#[tokio::test]
async fn restore_from_snapshot_restores_snapshot() {
    let origin_id = Id::new_v4();
    let current_id = Id::new_v4();
    let t0 = chrono::Utc::now().fixed_offset();
    let snapshot = TopicSnapshot {
        coaching_session_id: origin_id,
        body: "topic body".to_owned(),
        display_order: 3,
        priority: Some(entity::topic_priority::Priority::High),
        status: Status::Discussed,
        moved_from_session_id: None,
        deleted_at: None,
        updated_at: t0,
    };
    let moved = Model {
        coaching_session_id: current_id,
        status: Status::Open,
        moved_from_session_id: Some(origin_id),
        display_order: 0,
        undo_snapshot: Some(snapshot.clone()),
        ..topic(current_id, Id::new_v4(), 0)
    };
    let restored = Model {
        coaching_session_id: origin_id,
        status: Status::Discussed,
        display_order: 3,
        moved_from_session_id: None,
        updated_at: t0,
        undo_snapshot: None,
        ..moved.clone()
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // find_including_deleted_by_id → UPDATE.
        .append_query_results(vec![vec![moved.clone()]])
        .append_query_results(vec![vec![restored]])
        .into_connection();

    let result = restore_from_snapshot(&db, moved.id).await.unwrap().unwrap();
    assert_eq!(result.coaching_session_id, origin_id);
    assert_eq!(result.status, Status::Discussed);
    assert_eq!(result.display_order, 3);
    assert_eq!(result.moved_from_session_id, None);
    assert_eq!(result.updated_at, t0);
    assert_eq!(result.undo_snapshot, None);

    // The return-value asserts above only check the canned Mock row; verify the actual UPDATE
    // binds the SNAPSHOT's load-bearing values regardless of exact column count. Decode by
    // meaning rather than pinning a brittle positional vector.
    let updates = topic_update_value_rows(&db.into_transaction_log());
    assert_eq!(updates.len(), 1);
    let row = &updates[0];
    assert!(
        row.contains(&Value::from(origin_id)),
        "coaching_session_id should be the snapshot origin: {row:?}"
    );
    assert!(
        row.contains(&Value::from(3_i32)),
        "display_order should be the snapshot order: {row:?}"
    );
    // status binds the snapshot's status (Discussed), not the current Open.
    assert!(
        row.contains(&Value::from("discussed")),
        "status should bind the snapshot status (discussed), not open: {row:?}"
    );
    assert!(
        !row.contains(&Value::from("open")),
        "status must not bind the current open status: {row:?}"
    );
    // deleted_at binds NULL (a NULL timestamptz), un-deleting the row on a delete-undo.
    assert!(
        row.contains(&Value::ChronoDateTimeWithTimeZone(None)),
        "deleted_at should bind NULL: {row:?}"
    );
    // undo_snapshot is cleared to a JSON NULL.
    assert!(
        row.contains(&Value::Json(None)),
        "undo_snapshot should bind JSON NULL (cleared): {row:?}"
    );
    // WHERE id scopes the write to this topic.
    assert!(
        row.contains(&Value::from(moved.id)),
        "WHERE id should scope to this topic: {row:?}"
    );
}

/// restore_from_snapshot on a topic with no snapshot is a no-op: returns Ok(None), no UPDATE.
#[tokio::test]
async fn restore_from_snapshot_returns_none_without_snapshot() {
    let session_id = Id::new_v4();
    let settled = topic(session_id, Id::new_v4(), 0); // undo_snapshot: None

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![settled.clone()]])
        .into_connection();

    let result = restore_from_snapshot(&db, settled.id).await.unwrap();
    assert!(result.is_none());
    assert!(topic_update_value_rows(&db.into_transaction_log()).is_empty());
}

/// delete soft-deletes: ONE UPDATE that binds deleted_at -> a non-NULL timestamp AND
/// undo_snapshot -> a JSON object (not NULL), scoped to the topic id.
#[tokio::test]
async fn delete_soft_deletes_and_snapshots() {
    let session_id = Id::new_v4();
    let live = topic(session_id, Id::new_v4(), 0); // undo_snapshot/deleted_at: None
    let soft_deleted = Model {
        deleted_at: Some(chrono::Utc::now().fixed_offset()),
        ..live.clone()
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // find_by_id (live) → UPDATE.
        .append_query_results(vec![vec![live.clone()]])
        .append_query_results(vec![vec![soft_deleted]])
        .into_connection();

    delete(&db, live.id).await.unwrap();

    let updates = topic_update_value_rows(&db.into_transaction_log());
    assert_eq!(updates.len(), 1);
    let row = &updates[0];
    // deleted_at binds a non-NULL timestamp (teeth: missing if delete forgot to set it).
    assert!(
        row.iter()
            .any(|v| matches!(v, Value::ChronoDateTimeWithTimeZone(Some(_)))),
        "deleted_at should bind a non-NULL timestamp: {row:?}"
    );
    // undo_snapshot binds a JSON object (teeth: missing if delete forgot to snapshot).
    assert!(
        row.iter().any(|v| matches!(v, Value::Json(Some(_)))),
        "undo_snapshot should bind a JSON object: {row:?}"
    );
    // Scoped to this topic id.
    assert!(
        row.contains(&Value::from(live.id)),
        "WHERE id should scope to this topic: {row:?}"
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

/// move_deferred_to_session (hydration) re-parents a held Deferred topic forward and CLEARS its
/// snapshot: a batch hydration move is non-undoable, so any snapshot left by a prior defer_hold is
/// wiped (undo then returns 422 rather than time-traveling to the pre-defer state). Asserts on the
/// bound UPDATE values, not the canned Mock row, since MockDatabase echoes the row verbatim and
/// would mask a missing Set(None).
#[tokio::test]
async fn move_deferred_to_session_clears_snapshot() {
    let hold_id = Id::new_v4();
    let target_id = Id::new_v4();
    let origin_id = Id::new_v4();
    let snapshot = TopicSnapshot {
        coaching_session_id: origin_id,
        body: "topic body".to_owned(),
        display_order: 5,
        priority: Some(entity::topic_priority::Priority::High),
        status: Status::Discussed,
        moved_from_session_id: None,
        deleted_at: None,
        updated_at: chrono::Utc::now().fixed_offset(),
    };
    let held = Model {
        status: Status::Deferred,
        undo_snapshot: Some(snapshot.clone()),
        ..topic(hold_id, Id::new_v4(), 0)
    };
    // The canned row still carries the snapshot to prove the assertion reads the bound write,
    // not this echoed model.
    let moved_row = Model {
        coaching_session_id: target_id,
        status: Status::Open,
        moved_from_session_id: Some(hold_id),
        display_order: 0,
        ..held.clone()
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // source SELECT → target SELECT (empty, for base) → ONE UPDATE.
        .append_query_results(vec![vec![held.clone()]])
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .append_query_results(vec![vec![moved_row]])
        .into_connection();

    let moved = move_deferred_to_session(&db, hold_id, target_id)
        .await
        .unwrap();
    assert_eq!(moved.len(), 1);

    // Same SET-column layout as defer_move (coaching_session_id, display_order, status,
    // moved_from_session_id, undo_snapshot, updated_at, then WHERE id): undo_snapshot binds NULL.
    let updates = topic_update_value_rows(&db.into_transaction_log());
    assert_eq!(updates.len(), 1);
    let row = &updates[0];
    assert_eq!(row[0], Value::from(target_id)); // coaching_session_id -> target
    assert_eq!(row[2], Value::from("open")); // status -> open
    assert_eq!(row[3], Value::from(hold_id)); // moved_from_session_id -> source
    assert_eq!(
        row[4],
        Value::Json(None),
        "snapshot must be cleared: {row:?}"
    );
}

/// set_status is the settle point: a deliberate non-defer write CLEARS the snapshot. Asserts on the
/// bound UPDATE values, not the canned Mock row (which would echo whatever it was given).
#[tokio::test]
async fn set_status_clears_snapshot() {
    let session_id = Id::new_v4();
    let snapshot = TopicSnapshot {
        coaching_session_id: Id::new_v4(),
        body: "topic body".to_owned(),
        display_order: 1,
        priority: Some(entity::topic_priority::Priority::High),
        status: Status::Discussed,
        moved_from_session_id: None,
        deleted_at: None,
        updated_at: chrono::Utc::now().fixed_offset(),
    };
    let with_snapshot = Model {
        undo_snapshot: Some(snapshot),
        ..topic(session_id, Id::new_v4(), 0)
    };
    // Canned row still carries the snapshot to prove the assertion reads the bound write.
    let settled = with_snapshot.clone();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // find_by_id → UPDATE.
        .append_query_results(vec![vec![with_snapshot.clone()]])
        .append_query_results(vec![vec![settled]])
        .into_connection();

    set_status(&db, with_snapshot.id, Status::Open)
        .await
        .unwrap();

    // SET columns in entity order: status, undo_snapshot, updated_at, then WHERE id.
    let updates = topic_update_value_rows(&db.into_transaction_log());
    assert_eq!(updates.len(), 1);
    let row = &updates[0];
    assert_eq!(row[0], Value::from("open")); // status -> open
    assert_eq!(
        row[1],
        Value::Json(None),
        "snapshot must be cleared: {row:?}"
    );
}
