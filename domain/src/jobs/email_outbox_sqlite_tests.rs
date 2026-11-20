//! What the outbox sweep delivers and leaves behind, against a real SQL engine.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chrono::{NaiveDateTime, TimeDelta, Utc};
use entity_api::email_outbox::{Model, New, State};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set};
use serde_json::{json, Value};

use super::*;
use crate::test_utils::sqlite::{database, seed_user, within_time_limit};

/// A mailer that answers from a script and records every delivery.
#[derive(Clone, Default)]
struct Scripted {
    answers: Arc<Mutex<VecDeque<Receipt>>>,
    delivered: Arc<Mutex<Vec<(String, Value)>>>,
}

impl Scripted {
    fn answering(answers: impl IntoIterator<Item = Receipt>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(answers.into_iter().collect())),
            ..Self::default()
        }
    }

    fn delivered(&self) -> Vec<(String, Value)> {
        self.delivered.lock().expect("the log locks").clone()
    }
}

#[async_trait]
impl Mailer for Scripted {
    async fn deliver(&self, key: &str, body: &Value) -> Receipt {
        self.delivered
            .lock()
            .expect("the log locks")
            .push((key.to_string(), body.clone()));
        self.answers
            .lock()
            .expect("the script locks")
            .pop_front()
            .expect("a scripted answer is left")
    }
}

fn context(db: DatabaseConnection) -> Context {
    Context {
        db: Arc::new(db),
        config: Config::from_args(["test"]),
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

/// Writes one email to `recipient` in conversation `subject`, as of `at`.
async fn write(ctx: &Context, recipient: Id, subject: &str, label: &str, at: NaiveDateTime) {
    email_outbox::insert(
        &*ctx.db,
        vec![New {
            kind: "test".to_string(),
            subject_key: subject.to_string(),
            recipient_user_id: recipient,
            request: json!({ "label": label }),
        }],
        at,
    )
    .await
    .expect("the row inserts");
}

async fn rows(ctx: &Context) -> Vec<Model> {
    entity::email_outbox::Entity::find()
        .all(&*ctx.db)
        .await
        .expect("the rows read")
}

async fn only_row(ctx: &Context) -> Model {
    let mut all = rows(ctx).await;
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}

/// Makes a released row due again without waiting out its backoff.
async fn due_now(ctx: &Context, row: Model) {
    let mut row = row.into_active_model();
    row.not_before = Set(now());
    row.update(&*ctx.db).await.expect("the row updates");
}

async fn one_row(
    answers: impl IntoIterator<Item = Receipt>,
) -> (Context, Sweep<Scripted>, Scripted) {
    let db = database().await;
    let user = seed_user(&db).await;
    let ctx = context(db);
    write(&ctx, user, "user/a", "only", now()).await;
    let mailer = Scripted::answering(answers);
    (
        ctx,
        Sweep::new(mailer.clone(), RetryPolicy::default()),
        mailer,
    )
}

#[tokio::test]
async fn an_accepted_email_is_sent_under_its_key_and_its_body_dropped() {
    within_time_limit(async {
        let (ctx, sweep, mailer) = one_row([Receipt::Accepted]).await;
        let id = only_row(&ctx).await.id;

        sweep.run(&ctx).await.expect("the tick runs");

        let row = only_row(&ctx).await;
        assert_eq!(
            (row.state, row.request, row.claim_id),
            (State::Sent, None, None)
        );
        assert!(row.finished_at.is_some());
        assert_eq!(
            mailer.delivered(),
            [(idempotency_key(id), json!({ "label": "only" }))]
        );
    })
    .await;
}

#[tokio::test]
async fn a_retried_email_waits_then_goes_again_under_the_same_key() {
    within_time_limit(async {
        let (ctx, sweep, mailer) = one_row([retry(), Receipt::Accepted]).await;
        let before = now();

        sweep.run(&ctx).await.expect("the first tick runs");

        let row = only_row(&ctx).await;
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
        assert_eq!(only_row(&ctx).await.state, State::Sent);
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

        sweep.run(&ctx).await.expect("the tick runs");

        let row = only_row(&ctx).await;
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
        let ctx = context(db);
        let max_age = RetryPolicy::default().max_age;
        write(&ctx, user, "user/a", "stale", now() - max_age).await;
        let mailer = Scripted::default();

        Sweep::new(mailer.clone(), RetryPolicy::default())
            .run(&ctx)
            .await
            .expect("the tick runs");

        assert_eq!(only_row(&ctx).await.state, State::Failed);
        assert!(mailer.delivered().is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_newer_email_is_sent_after_the_older_one_finishes() {
    within_time_limit(async {
        let db = database().await;
        let user = seed_user(&db).await;
        let ctx = context(db);
        write(&ctx, user, "user/a", "older", now() - TimeDelta::seconds(1)).await;
        write(&ctx, user, "user/a", "newer", now()).await;
        let mailer = Scripted::answering([retry(), Receipt::Accepted, Receipt::Accepted]);
        let sweep = Sweep::new(mailer.clone(), RetryPolicy::default());

        sweep.run(&ctx).await.expect("the first tick runs");
        let older = rows(&ctx)
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
            .map(|(_, body)| body["label"].clone())
            .collect();
        assert_eq!(labels, [json!("older"), json!("older"), json!("newer")]);
        assert!(rows(&ctx).await.iter().all(|row| row.state == State::Sent));
    })
    .await;
}
