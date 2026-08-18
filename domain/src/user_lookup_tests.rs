use super::*;
use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

fn attempt(requester_user_id: Id) -> user_lookup_attempts::Model {
    user_lookup_attempts::Model {
        id: Id::new_v4(),
        requester_user_id,
        attempted_at: Utc::now().into(),
    }
}

fn exec() -> MockExecResult {
    MockExecResult {
        last_insert_id: 0,
        rows_affected: 0,
    }
}

/// Every statement the mock saw, in order.
fn statements(db: sea_orm::DatabaseConnection) -> Vec<String> {
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

/// Counting and inserting are two statements, so without the lock a burst of
/// concurrent lookups all read a count below the cap before any row lands and all
/// proceed. That is the case the cap exists to stop, and no behavioural assertion
/// catches it: `MockDatabase` cannot interleave transactions, so the mechanism is
/// what has to be pinned.
#[tokio::test]
async fn the_gate_locks_the_requester_before_counting() -> Result<(), Error> {
    let requester_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results([exec()])
        .append_query_results([Vec::<user_lookup_attempts::Model>::new()])
        .append_query_results([[attempt(requester_id)]])
        .into_connection();

    record_attempt_or_reject(&db, requester_id).await?;

    let sql = statements(db);
    let lock = sql
        .iter()
        .position(|statement| statement.contains("pg_advisory_xact_lock"))
        .expect("the requester must be locked");
    let count = sql
        .iter()
        .position(|statement| {
            statement.starts_with("SELECT") && statement.contains("user_lookup_attempts")
        })
        .expect("the window must be counted");
    let commit = sql
        .iter()
        .position(|statement| statement == "COMMIT")
        .expect("the gate must commit");

    assert!(lock < count, "the lock must precede the count: {sql:?}");
    assert!(
        count < commit,
        "the count and the insert must share the lock's transaction: {sql:?}"
    );

    Ok(())
}

/// A refused attempt must not be recorded, or a caller who keeps trying extends the
/// window they are being refused by and can never recover.
#[tokio::test]
async fn a_refused_attempt_is_not_recorded() {
    let requester_id = Id::new_v4();
    let over = (0..MAX_ATTEMPTS_PER_WINDOW)
        .map(|_| attempt(requester_id))
        .collect::<Vec<_>>();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results([exec()])
        .append_query_results([over])
        .into_connection();

    record_attempt_or_reject(&db, requester_id)
        .await
        .expect_err("over the cap must be refused");

    // No INSERT primed: had the refusal recorded anyway it would run past the end of
    // the queue and fail differently.
    let sql = statements(db);
    assert!(
        !sql.iter().any(|statement| statement.starts_with("INSERT")),
        "a refusal must not extend its own window: {sql:?}"
    );
}

/// Retention shorter than the rate-limit window would delete rows the next check
/// still needs, silently resetting a throttled requester's allowance.
#[tokio::test]
async fn the_sweep_refuses_retention_shorter_than_the_window() {
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    sweep_old_attempts(&db, WINDOW_HOURS)
        .await
        .expect_err("retention equal to the window must be refused");
    sweep_old_attempts(&db, WINDOW_HOURS - 1)
        .await
        .expect_err("retention shorter than the window must be refused");
}
