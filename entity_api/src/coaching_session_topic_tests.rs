use super::*;
use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

/// Builder for a topic Model with arbitrary body/timestamps.
fn topic(session_id: Id, id: Id, order: i32) -> Model {
    let now = chrono::Utc::now();
    Model {
        id,
        coaching_session_id: session_id,
        body: "topic body".to_owned(),
        user_id: Id::new_v4(),
        display_order: order,
        relevance: entity::topic_relevance::Relevance::Neutral,
        immediacy: entity::topic_immediacy::Immediacy::Neutral,
        created_at: now.into(),
        updated_at: now.into(),
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
    // Rating axes ARE on the wire.
    assert!(value.get("relevance").is_some());
    assert!(value.get("immediacy").is_some());
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
            r#"SELECT "coaching_session_topics"."id", "coaching_session_topics"."coaching_session_id", "coaching_session_topics"."body", "coaching_session_topics"."user_id", "coaching_session_topics"."display_order", CAST("coaching_session_topics"."relevance" AS "text"), CAST("coaching_session_topics"."immediacy" AS "text"), "coaching_session_topics"."created_at", "coaching_session_topics"."updated_at" FROM "refactor_platform"."coaching_session_topics" WHERE "coaching_session_topics"."coaching_session_id" = $1 ORDER BY "coaching_session_topics"."display_order" ASC, "coaching_session_topics"."created_at" ASC"#,
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
