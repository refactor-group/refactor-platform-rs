use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult, TransactionTrait};

use super::*;
use crate::test_utils::email_outbox::outgoing_email;

fn envelope(subject: Subject, recipient_id: Id) -> Envelope {
    Envelope {
        kind: "test",
        subject,
        recipient_id,
        email: outgoing_email("test"),
    }
}

#[tokio::test]
async fn enqueue_locks_each_conversation_once_before_writing() {
    let done = MockExecResult {
        last_insert_id: 0,
        rows_affected: 1,
    };
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results([done.clone(), done.clone(), done])
        .into_connection();
    let (relationship, coachee, coach) = (Id::new_v4(), Id::new_v4(), Id::new_v4());

    let txn = db.begin().await.expect("the transaction begins");
    enqueue(
        &txn,
        [
            envelope(Subject::CoachingRelationship(relationship), coachee),
            envelope(Subject::CoachingRelationship(relationship), coachee),
            envelope(Subject::CoachingRelationship(relationship), coach),
        ],
    )
    .await
    .expect("the emails are enqueued");
    txn.commit().await.expect("the transaction commits");

    let statements: Vec<String> = db
        .into_transaction_log()
        .iter()
        .flat_map(|transaction| transaction.statements())
        .map(|statement| statement.sql.clone())
        .collect();
    assert_eq!(statements.len(), 5, "{statements:?}");
    assert_eq!(statements[0], "BEGIN", "{statements:?}");
    assert!(
        statements[1..3]
            .iter()
            .all(|sql| sql.contains("pg_advisory_xact_lock")),
        "one lock per conversation comes first, inside the transaction: {statements:?}"
    );
    assert!(statements[3].starts_with("INSERT INTO"), "{statements:?}");
    assert_eq!(statements[4], "COMMIT", "{statements:?}");
}
