use super::*;
use chrono::Utc;
use sea_orm::{DatabaseBackend, MockDatabase, Value};
use std::collections::BTreeMap;

// `MarkViewed` is a `FromQueryResult` (not a `Model`), so canned rows are column maps.
fn row(
    previous_last_viewed_at: Option<DateTimeWithTimeZone>,
    last_viewed_at: DateTimeWithTimeZone,
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            "previous_last_viewed_at".to_owned(),
            Value::from(previous_last_viewed_at),
        ),
        ("last_viewed_at".to_owned(), Value::from(last_viewed_at)),
    ])
}

// Canned row -> MarkViewed: the atomic upsert returns the new marker plus the prior value.
#[tokio::test]
async fn mark_viewed_returns_new_and_prior_markers() {
    let coaching_session_id = Id::new_v4();
    let user_id = Id::new_v4();
    let now: DateTimeWithTimeZone = Utc::now().into();
    let prior: DateTimeWithTimeZone = (Utc::now() - chrono::Duration::hours(1)).into();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![row(Some(prior), now)]])
        .into_connection();

    let result = mark_viewed(&db, coaching_session_id, user_id)
        .await
        .expect("mark_viewed should map the canned row");
    assert_eq!(result.last_viewed_at, now);
    assert_eq!(result.previous_last_viewed_at, Some(prior));
}

// SQL-shape teeth: the single statement must snapshot the prior value via a CTE, upsert on
// the (user, session) conflict, RETURN both markers, and bind [session, user] in that order.
// A change that drops the CTE or returns the new value instead of the prior fails here.
#[tokio::test]
async fn mark_viewed_emits_atomic_prior_snapshot_sql() {
    let coaching_session_id = Id::new_v4();
    let user_id = Id::new_v4();
    let now: DateTimeWithTimeZone = Utc::now().into();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![row(None, now)]])
        .into_connection();

    let _ = mark_viewed(&db, coaching_session_id, user_id).await;

    let log = db.into_transaction_log();
    assert_eq!(log.len(), 1, "expected exactly one statement, got {log:?}");

    let stmt = log[0]
        .statements()
        .first()
        .cloned()
        .expect("log entry must carry a statement");

    let expected_sql = r#"WITH prev AS (
               SELECT last_viewed_at
               FROM refactor_platform.coaching_session_views
               WHERE coaching_session_id = $1 AND user_id = $2
           )
           INSERT INTO refactor_platform.coaching_session_views
               (coaching_session_id, user_id, last_viewed_at, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW(), NOW())
           ON CONFLICT (user_id, coaching_session_id)
           DO UPDATE SET last_viewed_at = NOW(), updated_at = NOW()
           RETURNING last_viewed_at, (SELECT last_viewed_at FROM prev) AS previous_last_viewed_at"#;
    assert_eq!(stmt.sql, expected_sql, "exact upsert SQL must not drift");

    // Belt-and-suspenders substring checks on the load-bearing clauses.
    assert!(stmt.sql.contains("WITH prev AS"));
    assert!(stmt.sql.contains("SELECT last_viewed_at FROM prev"));
    assert!(stmt
        .sql
        .contains("ON CONFLICT (user_id, coaching_session_id)\n           DO UPDATE"));
    assert!(stmt.sql.contains("RETURNING last_viewed_at"));
    assert!(stmt.sql.contains("previous_last_viewed_at"));

    // Binds exactly [coaching_session_id, user_id] in that order.
    let values = stmt.values.expect("statement must carry bound values").0;
    assert_eq!(values.len(), 2, "expected two binds");
    assert_eq!(values[0], Value::from(coaching_session_id));
    assert_eq!(values[1], Value::from(user_id));
}
