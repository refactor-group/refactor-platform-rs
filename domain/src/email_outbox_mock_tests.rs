use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

use super::*;
use crate::gateway::resend::SendEmailRequestBuilder;

async fn envelope(subject: Subject, recipient_id: Id) -> Envelope {
    Envelope {
        kind: "test",
        subject,
        recipient_id,
        request: SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("recipient@example.com", "Recipient")
            .template_id("t")
            .build()
            .await
            .expect("the request builds"),
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

    enqueue(
        &db,
        [
            envelope(Subject::CoachingRelationship(relationship), coachee).await,
            envelope(Subject::CoachingRelationship(relationship), coachee).await,
            envelope(Subject::CoachingRelationship(relationship), coach).await,
        ],
    )
    .await
    .expect("the emails are enqueued");

    let statements: Vec<String> = db
        .into_transaction_log()
        .iter()
        .flat_map(|transaction| transaction.statements())
        .map(|statement| statement.sql.clone())
        .collect();
    assert_eq!(statements.len(), 3, "{statements:?}");
    assert!(
        statements[..2]
            .iter()
            .all(|sql| sql.contains("pg_advisory_xact_lock")),
        "one lock per conversation comes first: {statements:?}"
    );
    assert!(statements[2].starts_with("INSERT INTO"), "{statements:?}");
}
