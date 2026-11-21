//! What the outbox sweep delivers and leaves behind, against a real SQL engine.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chrono::{NaiveDateTime, TimeDelta, Utc};
use entity_api::email_outbox::{Model, New, State};
use sea_orm::{ActiveModelTrait, IntoActiveModel, Set};
use serde_json::{json, Value};
use service::config::Config;

use super::*;
use crate::test_utils::email_outbox::{only_row, outgoing_email, rows};
use crate::test_utils::jobs::job_context;
use crate::test_utils::sqlite::{database, seed_user, within_time_limit};

/// A mailer that answers from a script and records every delivery.
#[derive(Clone, Default)]
struct Scripted {
    answers: Arc<Mutex<VecDeque<Receipt>>>,
    delivered: Arc<Mutex<Vec<(String, OutgoingEmail)>>>,
}

impl Scripted {
    fn answering(answers: impl IntoIterator<Item = Receipt>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(answers.into_iter().collect())),
            ..Self::default()
        }
    }

    fn delivered(&self) -> Vec<(String, OutgoingEmail)> {
        self.delivered.lock().expect("the log locks").clone()
    }
}

#[async_trait]
impl Mailer for Scripted {
    async fn deliver(&self, key: &str, email: &OutgoingEmail) -> Receipt {
        self.delivered
            .lock()
            .expect("the log locks")
            .push((key.to_string(), email.clone()));
        self.answers
            .lock()
            .expect("the script locks")
            .pop_front()
            .expect("a scripted answer is left")
    }
}

fn now() -> NaiveDateTime {
    Utc::now().naive_utc()
}

fn retry() -> Receipt {
    Receipt::Retry {
        after: None,
        error: "503 - down".to_string(),
    }
}

/// Writes one email titled `label` to `recipient` in conversation `subject`, as of `at`.
async fn write(ctx: &Context, recipient: Id, subject: &str, label: &str, at: NaiveDateTime) {
    let stored = serde_json::to_value(outgoing_email(label)).expect("the email serializes");
    store(ctx, recipient, subject, stored, at).await;
}

/// Stores `request` as one row's body, whatever its shape.
async fn store(ctx: &Context, recipient: Id, subject: &str, request: Value, at: NaiveDateTime) {
    email_outbox::insert(
        &*ctx.db,
        vec![New {
            kind: "test".to_string(),
            subject_key: subject.to_string(),
            recipient_user_id: recipient,
            request,
        }],
        at,
    )
    .await
    .expect("the row inserts");
}

/// A sweep delivering through `mailer`.
fn sending(mailer: &Scripted) -> Sweep {
    Sweep::new(Arc::new(mailer.clone()), RetryPolicy::default())
}

/// Runs one tick; returns what it handled of what it found.
async fn tick(sweep: &Sweep, ctx: &Context) -> (u64, u64) {
    let outcome = sweep.run(ctx).await.expect("the tick runs");
    (outcome.processed, outcome.attempted)
}

/// Makes a released row due again without waiting out its backoff.
async fn due_now(ctx: &Context, row: Model) {
    let mut row = row.into_active_model();
    row.not_before = Set(now());
    row.update(&*ctx.db).await.expect("the row updates");
}

async fn one_row(answers: impl IntoIterator<Item = Receipt>) -> (Context, Sweep, Scripted) {
    let db = database().await;
    let user = seed_user(&db).await;
    let ctx = job_context(db);
    write(&ctx, user, "user/a", "only", now()).await;
    let mailer = Scripted::answering(answers);
    (ctx, sending(&mailer), mailer)
}

#[tokio::test]
async fn an_accepted_email_is_sent_under_its_key_and_its_body_dropped() {
    within_time_limit(async {
        let (ctx, sweep, mailer) = one_row([Receipt::Accepted]).await;
        let id = only_row(&*ctx.db).await.id;

        assert_eq!(tick(&sweep, &ctx).await, (1, 1), "a sent email is handled");

        let row = only_row(&*ctx.db).await;
        assert_eq!(
            (row.state, row.request, row.claim_id),
            (State::Sent, None, None)
        );
        assert!(row.finished_at.is_some());
        assert_eq!(
            mailer.delivered(),
            [(idempotency_key(id), outgoing_email("only"))]
        );
    })
    .await;
}

#[tokio::test]
async fn a_retried_email_waits_then_goes_again_under_the_same_key() {
    within_time_limit(async {
        let (ctx, sweep, mailer) = one_row([retry(), Receipt::Accepted]).await;
        let before = now();

        assert_eq!(
            tick(&sweep, &ctx).await,
            (0, 1),
            "a retry leaves the tick short"
        );

        let row = only_row(&*ctx.db).await;
        assert_eq!(row.state, State::Pending);
        assert!(
            row.not_before >= before + RetryPolicy::default().base,
            "{row:?}"
        );
        assert_eq!(row.last_error.as_deref(), Some("503 - down"));
        sweep.run(&ctx).await.expect("the waiting tick runs");
        assert_eq!(mailer.delivered().len(), 1, "it waits out its backoff");

        due_now(&ctx, row).await;
        sweep.run(&ctx).await.expect("the second tick runs");

        let delivered = mailer.delivered();
        assert_eq!(only_row(&*ctx.db).await.state, State::Sent);
        assert_eq!(delivered.len(), 2);
        assert_eq!(
            delivered[0], delivered[1],
            "the repeat is the same key and body"
        );
    })
    .await;
}

#[tokio::test]
async fn a_rejected_email_fails() {
    within_time_limit(async {
        let (ctx, sweep, _) = one_row([Receipt::Rejected("422 - bad".to_string())]).await;

        assert_eq!(
            tick(&sweep, &ctx).await,
            (0, 1),
            "a rejection leaves the tick short"
        );

        let row = only_row(&*ctx.db).await;
        assert_eq!(
            (row.state, row.request, row.last_error.as_deref()),
            (State::Failed, None, Some("422 - bad"))
        );
    })
    .await;
}

#[tokio::test]
async fn an_expired_email_fails_without_a_delivery() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        let ctx = job_context(db);
        let max_age = RetryPolicy::default().max_age;
        write(&ctx, user, "user/a", "stale", now() - max_age).await;
        let mailer = Scripted::default();

        let expired = tick(&sending(&mailer), &ctx).await;

        assert_eq!(expired, (0, 1), "an expired email leaves the tick short");
        assert_eq!(only_row(&*ctx.db).await.state, State::Failed);
        assert!(mailer.delivered().is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_newer_email_is_sent_after_the_older_one_finishes() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        let ctx = job_context(db);
        write(&ctx, user, "user/a", "older", now() - TimeDelta::seconds(1)).await;
        write(&ctx, user, "user/a", "newer", now()).await;
        let mailer = Scripted::answering([retry(), Receipt::Accepted, Receipt::Accepted]);
        let sweep = sending(&mailer);

        sweep.run(&ctx).await.expect("the first tick runs");
        let older = rows(&*ctx.db)
            .await
            .into_iter()
            .find(|row| row.last_error.is_some())
            .expect("the older row was tried");
        due_now(&ctx, older).await;
        sweep.run(&ctx).await.expect("the second tick runs");
        sweep.run(&ctx).await.expect("the third tick runs");

        let labels: Vec<_> = mailer
            .delivered()
            .iter()
            .map(|(_, email)| email.subject.clone())
            .collect();
        assert_eq!(labels, ["older", "older", "newer"]);
        assert!(rows(&*ctx.db)
            .await
            .iter()
            .all(|row| row.state == State::Sent));
    })
    .await;
}

#[tokio::test]
async fn without_a_mailer_due_emails_are_dropped_unsent() {
    within_time_limit(async {
        let (ctx, _, _) = one_row([]).await;

        let drained = tick(&Sweep::draining(RetryPolicy::default()), &ctx).await;

        let row = only_row(&*ctx.db).await;
        assert_eq!(drained, (1, 1), "draining is not a failing tick");
        assert_eq!(
            (row.state, row.request, row.last_error.as_deref()),
            (State::Failed, None, Some("email is not configured"))
        );
    })
    .await;
}

#[tokio::test]
async fn an_unreadable_stored_email_fails_without_a_delivery() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        let ctx = job_context(db);
        store(&ctx, user, "user/a", json!({ "template": "t" }), now()).await;
        let mailer = Scripted::default();

        let unreadable = tick(&sending(&mailer), &ctx).await;

        let row = only_row(&*ctx.db).await;
        assert_eq!(
            unreadable,
            (0, 1),
            "an unreadable email leaves the tick short"
        );
        assert_eq!(
            (row.state, row.request, row.last_error.as_deref()),
            (State::Failed, None, Some("stored email could not be read"))
        );
        assert!(mailer.delivered().is_empty());
    })
    .await;
}

fn config(args: &[&str]) -> Config {
    Config::from_args(["test"].iter().chain(args))
}

/// Whether `result` is the configuration error a refused start reports.
fn refused_for_config(result: &Result<Sweep, Error>) -> bool {
    matches!(
        result,
        Err(Error {
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
            ..
        })
    )
}

#[tokio::test]
async fn production_refuses_to_start_without_an_email_provider() {
    let refused = Sweep::from_config(&config(&[
        "--runtime-env=production",
        "--frontend-base-url=https://app.example.com",
    ]))
    .await;

    assert!(refused_for_config(&refused), "{:?}", refused.err());
}

#[tokio::test]
async fn outside_production_no_email_provider_drains() {
    let drained = Sweep::from_config(&config(&["--runtime-env=staging"]))
        .await
        .expect("staging starts without a provider");

    assert!(drained.mailer.is_none());
}

#[tokio::test]
async fn a_configured_email_provider_delivers_in_production() {
    let sweep = Sweep::from_config(&config(&[
        "--runtime-env=production",
        "--resend-api-key=test_key",
        "--frontend-base-url=https://app.example.com",
    ]))
    .await
    .expect("production starts with a provider");

    assert!(sweep.mailer.is_some());
}

#[tokio::test]
async fn production_refuses_to_start_without_a_frontend_for_composed_emails() {
    let refused = Sweep::from_config(&config(&[
        "--runtime-env=production",
        "--resend-api-key=test_key",
    ]))
    .await;

    assert!(refused_for_config(&refused), "{:?}", refused.err());
}

#[tokio::test]
async fn outside_production_no_frontend_still_starts() {
    let sweep = Sweep::from_config(&config(&[
        "--runtime-env=staging",
        "--resend-api-key=test_key",
    ]))
    .await
    .expect("staging starts without a frontend");

    assert!(sweep.mailer.is_some());
}
