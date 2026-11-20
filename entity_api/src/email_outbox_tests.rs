use chrono::{NaiveDate, TimeDelta};
use sea_orm::{DatabaseBackend, MockDatabase};

use super::{claim, Model};
use crate::Id;

#[tokio::test]
async fn claim_locks_rows_with_skip_locked() {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![Vec::<Model>::new()])
        .into_connection();
    let now = NaiveDate::from_ymd_opt(2026, 12, 1)
        .and_then(|day| day.and_hms_opt(12, 0, 0))
        .expect("valid test time");

    let claimed = claim(&db, now, TimeDelta::minutes(5), 10, Id::new_v4())
        .await
        .expect("the claim runs");
    assert!(claimed.is_empty());

    let log = format!("{:?}", db.into_transaction_log());
    let (outer, subquery) = log
        .split_once(" IN (SELECT ")
        .expect("the claim selects its rows in a subquery");
    let subquery = subquery
        .split_once(" RETURNING ")
        .map(|(inner, _)| inner)
        .expect("the claim returns the rows it took");
    assert!(outer.contains("UPDATE "), "the claim is one update: {log}");
    assert!(
        subquery.contains(r#"FROM \"refactor_platform\".\"email_outbox\" AS \"older\""#),
        "the ordering check reads the schema's table: {subquery}"
    );
    assert!(
        subquery.contains(r#"ORDER BY \"created_at\" ASC, \"position\" ASC, \"id\" ASC LIMIT"#),
        "the claim takes rows in total order: {subquery}"
    );
    assert!(
        subquery.contains(" LIMIT $") && subquery.contains(" FOR UPDATE SKIP LOCKED) AND "),
        "the subquery must lock its rows and skip locked ones: {subquery}"
    );
}
