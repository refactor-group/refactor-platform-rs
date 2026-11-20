//! Which outbox rows a claim takes and what each transition leaves behind, against a real
//! SQL engine.

use chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use entity_api::email_outbox::{self, Claimed, New, State};
use sea_orm::{DatabaseConnection, TransactionTrait};
use serde_json::json;

use super::*;
use crate::test_utils::email_outbox::{only_row, outgoing_email, rows};
use crate::test_utils::sqlite::{database, seed_user, within_time_limit};

const LEASE: TimeDelta = TimeDelta::minutes(5);

fn at(minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 12, 1)
        .and_then(|day| day.and_hms_opt(12, minute, 0))
        .expect("valid test time")
}

fn new(subject: &str, recipient: Id, label: &str) -> New {
    New {
        kind: "test".to_string(),
        subject_key: subject.to_string(),
        recipient_user_id: recipient,
        request: json!({ "label": label }),
    }
}

/// Writes one batch at `now`.
async fn write(db: &DatabaseConnection, rows: Vec<New>, now: NaiveDateTime) {
    email_outbox::insert(db, rows, now)
        .await
        .expect("the rows insert");
}

/// Claims every eligible row at `now`; returns their labels, sorted.
async fn claim_labels(db: &DatabaseConnection, now: NaiveDateTime, limit: u64) -> Vec<String> {
    let mut labels: Vec<String> = claim(db, now, limit)
        .await
        .iter()
        .map(|claimed| {
            claimed.request["label"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    labels.sort();
    labels
}

/// The held row whose body carries `label`.
fn labelled<'a>(held: &'a [Claimed], label: &str) -> &'a Claimed {
    held.iter()
        .find(|claimed| claimed.request["label"] == label)
        .expect("the row is held")
}

async fn claim(db: &DatabaseConnection, now: NaiveDateTime, limit: u64) -> Vec<Claimed> {
    email_outbox::claim(db, now, LEASE, limit, Id::new_v4())
        .await
        .expect("the claim runs")
}

fn envelope(recipient_id: Id, kind: &'static str) -> Envelope {
    Envelope {
        kind,
        subject: Subject::User(recipient_id),
        recipient_id,
        email: outgoing_email(kind),
    }
}

#[tokio::test]
async fn enqueue_writes_with_the_callers_transaction() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;

        let rolled_back = db.begin().await?;
        enqueue(&rolled_back, [envelope(user, "dropped")]).await?;
        rolled_back.rollback().await?;
        assert!(
            rows(&db).await.is_empty(),
            "a rolled-back change owes no email"
        );

        let committed = db.begin().await?;
        let written = enqueue(
            &committed,
            [envelope(user, "first"), envelope(user, "second")],
        )
        .await?;
        committed.commit().await?;

        let stored = rows(&db).await;
        assert_eq!(written, 2);
        assert_eq!(
            stored
                .iter()
                .map(|row| (row.kind.as_str(), row.position, row.state))
                .collect::<Vec<_>>(),
            [("first", 0, State::Pending), ("second", 1, State::Pending)]
        );
        assert_eq!(stored[0].subject_key, Subject::User(user).key());
        let first: OutgoingEmail =
            serde_json::from_value(stored[0].request.clone().unwrap_or_default())?;
        assert_eq!(
            first,
            outgoing_email("first"),
            "the stored email reads back whole"
        );
        Ok::<(), Box<dyn std::error::Error>>(())
    })
    .await
    .expect("the test runs");
}

#[tokio::test]
async fn claim_takes_due_rows_oldest_first_up_to_the_limit() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        write(&db, vec![new("user/b", user, "later")], at(2)).await;
        write(&db, vec![new("user/a", user, "earlier")], at(1)).await;
        write(&db, vec![new("user/c", user, "not yet")], at(8)).await;

        assert_eq!(claim_labels(&db, at(5), 1).await, ["earlier"]);
        assert_eq!(
            claim_labels(&db, at(5), 10).await,
            ["later"],
            "the first claim's lease still holds its row, and a row not yet due waits"
        );
        assert_eq!(
            claim_labels(&db, at(8), 10).await,
            ["not yet"],
            "a row is due once its not_before passes"
        );
        assert_eq!(
            claim_labels(&db, at(5) + LEASE, 10).await,
            ["earlier", "later"],
            "a lapsed lease frees its row"
        );
        assert_eq!(
            rows(&db)
                .await
                .iter()
                .map(|row| row.attempts)
                .collect::<Vec<_>>(),
            [2, 2, 1],
            "each claim counts an attempt"
        );
    })
    .await;
}

#[tokio::test]
async fn an_older_pending_row_holds_back_its_conversation_only() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        let other_user = seed_user(&db).await;
        write(&db, vec![new("user/a", user, "first")], at(1)).await;
        write(
            &db,
            vec![
                new("user/a", user, "behind first"),
                new("user/a", other_user, "other recipient"),
                new("user/b", user, "other subject"),
            ],
            at(2),
        )
        .await;

        assert_eq!(
            claim_labels(&db, at(5), 10).await,
            ["first", "other recipient", "other subject"]
        );
    })
    .await;
}

#[tokio::test]
async fn rows_written_at_the_same_instant_are_ordered_by_id() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        write(&db, vec![new("user/a", user, "one")], at(1)).await;
        write(&db, vec![new("user/a", user, "two")], at(1)).await;
        let first = rows(&db)
            .await
            .into_iter()
            .map(|row| row.id)
            .min()
            .expect("two rows");

        let claimed = claim(&db, at(5), 10).await;

        assert_eq!(
            claimed.iter().map(|row| row.id).collect::<Vec<_>>(),
            [first],
            "only the lower id of a tie goes first"
        );
    })
    .await;
}

#[tokio::test]
async fn a_batch_is_claimed_in_the_order_it_was_written() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        let other_user = seed_user(&db).await;
        write(
            &db,
            vec![
                new("user/a", user, "one"),
                new("user/a", user, "two"),
                new("user/b", other_user, "three"),
            ],
            at(1),
        )
        .await;

        assert_eq!(claim_labels(&db, at(5), 10).await, ["one", "three"]);
        let held = claim(&db, at(5) + LEASE, 10).await;
        let first = held
            .iter()
            .find(|claimed| claimed.request["label"] == "one")
            .expect("the first row is claimed again");
        email_outbox::mark_sent(&db, first, at(5) + LEASE)
            .await
            .expect("it is sent");

        assert_eq!(
            claim_labels(&db, at(5) + LEASE * 2, 10).await,
            ["three", "two"],
            "the second waits until the first of its batch finishes"
        );
    })
    .await;
}

#[tokio::test]
async fn a_failed_older_row_no_longer_holds_back_its_conversation() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        write(&db, vec![new("user/a", user, "first")], at(1)).await;
        write(&db, vec![new("user/a", user, "second")], at(2)).await;

        let held = claim(&db, at(5), 10).await;
        email_outbox::fail(&db, &held[0], at(5), "422")
            .await
            .expect("it fails");

        assert_eq!(claim_labels(&db, at(5), 10).await, ["second"]);
    })
    .await;
}

#[tokio::test]
async fn finishing_drops_the_body_and_the_claim() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        let other_user = seed_user(&db).await;
        write(
            &db,
            vec![
                new("user/a", user, "sent"),
                new("user/b", other_user, "failed"),
            ],
            at(1),
        )
        .await;
        let held = claim(&db, at(5), 10).await;

        assert!(email_outbox::mark_sent(&db, labelled(&held, "sent"), at(6))
            .await
            .expect("it is sent"));
        assert!(
            email_outbox::fail(&db, labelled(&held, "failed"), at(7), "422 - refused")
                .await
                .expect("it fails")
        );

        let finished = rows(&db).await;
        assert_eq!(
            finished
                .iter()
                .map(|row| (
                    row.state,
                    row.request.clone(),
                    row.claim_id,
                    row.lease_until,
                    row.finished_at.map(|at| at.naive_utc()),
                    row.last_error.clone()
                ))
                .collect::<Vec<_>>(),
            [
                (State::Sent, None, None, None, Some(at(6)), None),
                (
                    State::Failed,
                    None,
                    None,
                    None,
                    Some(at(7)),
                    Some("422 - refused".to_string())
                ),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn release_hands_the_row_back_for_a_later_attempt() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        write(&db, vec![new("user/a", user, "retried")], at(1)).await;
        let held = claim(&db, at(5), 10).await;

        assert!(email_outbox::release(&db, &held[0], at(5), at(9), "503")
            .await
            .expect("it is released"));

        let row = only_row(&db).await;
        assert_eq!(
            (row.state, row.claim_id, row.lease_until, row.not_before),
            (State::Pending, None, None, at(9))
        );
        assert_eq!(row.last_error.as_deref(), Some("503"));
        assert!(row.request.is_some(), "a pending row keeps its body");
        assert!(
            claim(&db, at(8), 10).await.is_empty(),
            "it waits for its not_before"
        );
        assert_eq!(claim_labels(&db, at(9), 10).await, ["retried"]);
    })
    .await;
}

#[tokio::test]
async fn a_stale_claim_changes_nothing() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        write(&db, vec![new("user/a", user, "contested")], at(1)).await;
        let stale = claim(&db, at(5), 10).await.remove(0);
        let current = claim(&db, at(5) + LEASE, 10).await.remove(0);
        let before = only_row(&db).await;

        assert!(!email_outbox::mark_sent(&db, &stale, at(11))
            .await
            .expect("it runs"));
        assert!(!email_outbox::fail(&db, &stale, at(11), "x")
            .await
            .expect("it runs"));
        assert!(!email_outbox::release(&db, &stale, at(11), at(12), "x")
            .await
            .expect("it runs"));

        assert_eq!(only_row(&db).await, before);
        assert_eq!(before.claim_id, Some(current.claim_id));
    })
    .await;
}

#[tokio::test]
async fn purge_deletes_only_rows_finished_before_the_cutoff() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        let other_user = seed_user(&db).await;
        let third_user = seed_user(&db).await;
        write(
            &db,
            vec![
                new("user/a", user, "old"),
                new("user/b", other_user, "recent"),
                new("user/c", third_user, "pending"),
            ],
            at(1),
        )
        .await;
        let held = claim(&db, at(2), 2).await;
        email_outbox::mark_sent(&db, labelled(&held, "old"), at(3))
            .await
            .expect("it is sent");
        email_outbox::fail(&db, labelled(&held, "recent"), at(10), "x")
            .await
            .expect("it fails");

        let purged = email_outbox::purge(&db, at(10))
            .await
            .expect("the purge runs");

        assert_eq!(purged, 1);
        assert_eq!(
            rows(&db)
                .await
                .iter()
                .map(|row| row.state)
                .collect::<Vec<_>>(),
            [State::Failed, State::Pending]
        );
    })
    .await;
}
