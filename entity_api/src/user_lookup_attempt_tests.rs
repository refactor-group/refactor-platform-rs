use super::*;
use chrono::{Duration, Utc};
use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

fn attempt(requester_user_id: Id, age: Duration) -> Model {
    Model {
        id: Id::new_v4(),
        requester_user_id,
        attempted_at: (Utc::now() - age).into(),
    }
}

/// Every statement the mock saw, in order.
fn logged_sql(db: sea_orm::DatabaseConnection) -> Vec<String> {
    db.into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
        })
        .collect()
}

#[tokio::test]
async fn record_inserts_an_attempt_for_the_requester() -> Result<(), Error> {
    let requester_user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[attempt(requester_user_id, Duration::zero())]])
        .into_connection();

    let recorded = record(&db, requester_user_id).await?;

    assert_eq!(recorded.requester_user_id, requester_user_id);

    // `attempted_at` is left to the column default so the database clock stamps it.
    // Binding an application clock would let instances disagree about the window.
    let sql = logged_sql(db);
    assert!(
        !sql[0].contains("attempted_at"),
        "attempted_at must be left to the column default: {}",
        sql[0]
    );

    Ok(())
}

/// The limit is per requester, not global: one admin exhausting their allowance must
/// not throttle everyone else.
#[tokio::test]
async fn count_since_is_scoped_to_one_requester_and_the_window() -> Result<(), Error> {
    let requester_user_id = Id::new_v4();
    let since = (Utc::now() - Duration::hours(1)).into();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![
            attempt(requester_user_id, Duration::minutes(5)),
            attempt(requester_user_id, Duration::minutes(30)),
        ]])
        .into_connection();

    assert_eq!(count_since(&db, requester_user_id, since).await?, 2);

    let sql = logged_sql(db);
    let predicate = sql[0]
        .split_once(" WHERE ")
        .map(|(_, rest)| rest)
        .expect("the count must be filtered");
    assert!(
        predicate.contains("requester_user_id"),
        "the count must be scoped to one requester: {}",
        sql[0]
    );
    assert!(
        predicate.contains("attempted_at"),
        "the count must be scoped to the window: {}",
        sql[0]
    );

    Ok(())
}

#[tokio::test]
async fn count_since_reports_zero_for_a_requester_with_no_attempts() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<Model>::new()])
        .into_connection();

    assert_eq!(
        count_since(&db, Id::new_v4(), (Utc::now() - Duration::hours(1)).into()).await?,
        0
    );

    Ok(())
}

/// The sweep is why this table is not append-only. `user_role_changes` revokes
/// DELETE because it is a permanent record; this one is rate-limiter state and must
/// be prunable, so the delete has to be real and scoped by age alone.
#[tokio::test]
async fn delete_older_than_prunes_by_age_across_all_requesters() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 7,
        }])
        .into_connection();

    assert_eq!(
        delete_older_than(&db, (Utc::now() - Duration::days(1)).into()).await?,
        7
    );

    let sql = logged_sql(db);
    assert!(
        sql[0].starts_with("DELETE"),
        "the sweep must delete, not soft-delete: {}",
        sql[0]
    );
    let predicate = sql[0]
        .split_once(" WHERE ")
        .map(|(_, rest)| rest)
        .expect("the sweep must be bounded");
    assert!(
        predicate.contains("attempted_at"),
        "the sweep must be bounded by age: {}",
        sql[0]
    );
    assert!(
        !predicate.contains("requester_user_id"),
        "the sweep is global, not per requester: {}",
        sql[0]
    );

    Ok(())
}
