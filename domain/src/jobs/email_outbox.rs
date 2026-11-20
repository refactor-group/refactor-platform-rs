//! Delivers outbox rows and purges finished ones.
//!
//! The sweep is the only code that hands an outbox email to the provider. Each tick claims
//! due rows under a lease, fails those past the key window without an attempt, delivers
//! the rest a few at a time under each row's idempotency key, and records what each
//! receipt settles to. A row whose sender died comes back when its lease lapses, and the
//! key makes the repeat safe. Bodies are never logged. See
//! `docs/architecture/email_outbox.md`.
//!
//! Without a mailer the sweep still runs, failing due rows unsent, so pending bodies and
//! the sign-in links in them never pile up.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use entity_api::email_outbox::{self, Claimed};
use futures::future::ready;
use futures::stream::{self, StreamExt};
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::email_outbox::{idempotency_key, settle, Mailer, Receipt, RetryPolicy, Transition};
use crate::error::Error;
use crate::gateway::resend;
use crate::jobs::{Context, FirstRun, Job, Outcome};
use crate::Id;

/// How often the sweep looks for due rows.
const INTERVAL: Duration = Duration::from_secs(1);

/// Most rows one tick claims.
const BATCH: u64 = 10;

/// Most deliveries in flight at once.
const CONCURRENCY: usize = 5;

/// How often finished rows are purged.
const PURGE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How long a finished row is kept.
const RETENTION: TimeDelta = TimeDelta::days(7);

/// What a row drained without a mailer is failed with.
const NOT_CONFIGURED: &str = "email is not configured";

/// Delivers due outbox rows through `M`, or drains them unsent without one.
pub struct Sweep<M: Mailer> {
    mailer: Option<M>,
    policy: RetryPolicy,
}

impl<M: Mailer> Sweep<M> {
    /// A sweep that delivers through `mailer`.
    pub fn new(mailer: M, policy: RetryPolicy) -> Self {
        Self {
            mailer: Some(mailer),
            policy,
        }
    }

    /// A sweep that fails every due row unsent.
    pub fn draining(policy: RetryPolicy) -> Self {
        Self {
            mailer: None,
            policy,
        }
    }

    /// Settles one claimed row: drains it without a mailer, fails it unsent when expired,
    /// otherwise delivers it. True when the row was recorded as sent, or drained.
    async fn settle_one(&self, db: &DatabaseConnection, claimed: &Claimed) -> bool {
        let now = Utc::now().naive_utc();
        let (transition, handled) = match &self.mailer {
            None => {
                debug!(
                    "[email-outbox] {} email {} dropped: {NOT_CONFIGURED}",
                    claimed.kind, claimed.id
                );
                (Transition::Fail(NOT_CONFIGURED.to_string()), true)
            }
            Some(_) if self.policy.is_expired(claimed.created_at, now) => {
                error!(
                    "[email-outbox] {} email {} expired after {} attempt(s) and will not be sent",
                    claimed.kind,
                    claimed.id,
                    claimed.attempts - 1
                );
                (
                    Transition::Fail("expired before it could be sent".to_string()),
                    false,
                )
            }
            Some(mailer) => {
                let receipt = mailer
                    .deliver(&idempotency_key(claimed.id), &claimed.request)
                    .await;
                log_receipt(claimed, &receipt);
                let transition = settle(
                    &receipt,
                    claimed.attempts,
                    Utc::now().naive_utc(),
                    &self.policy,
                );
                let sent = transition == Transition::Sent;
                (transition, sent)
            }
        };
        record(db, claimed, transition).await && handled
    }
}

impl Sweep<resend::Client> {
    /// Builds the sweep over Resend, draining instead when Resend is not configured.
    pub async fn from_config(config: &Config) -> Self {
        let mailer = match config.resend_api_key() {
            None => None,
            Some(_) => resend::Client::new(config)
                .await
                .inspect_err(|e| {
                    warn!("[email-outbox] could not build the Resend client: {e:?}");
                })
                .ok(),
        };
        match mailer {
            Some(client) => {
                info!("[email-outbox] delivering through Resend");
                Self::new(client, RetryPolicy::default())
            }
            None => {
                info!("[email-outbox] {NOT_CONFIGURED}; due rows will be dropped unsent");
                Self::draining(RetryPolicy::default())
            }
        }
    }
}

#[async_trait]
impl<M: Mailer> Job for Sweep<M> {
    fn name(&self) -> &'static str {
        "email-outbox"
    }

    fn interval(&self) -> Duration {
        INTERVAL
    }

    /// Rows written while the process was down are owed now.
    fn first_run(&self) -> FirstRun {
        FirstRun::Immediately
    }

    async fn run(&self, ctx: &Context) -> Result<Outcome, Error> {
        let claimed = email_outbox::claim(
            &*ctx.db,
            Utc::now().naive_utc(),
            self.policy.lease,
            BATCH,
            Id::new_v4(),
        )
        .await?;

        // A retried or failed send leaves the tick short, so a failing Resend shows at WARN.
        let attempted = claimed.len() as u64;
        let handled = stream::iter(claimed)
            .map(|row| async move { self.settle_one(&ctx.db, &row).await })
            .buffer_unordered(CONCURRENCY)
            .fold(0, |count, handled| ready(count + u64::from(handled)))
            .await;

        Ok(Outcome::partial(handled, attempted))
    }
}

/// Logs a delivery that did not succeed, by row and kind.
fn log_receipt(claimed: &Claimed, receipt: &Receipt) {
    match receipt {
        Receipt::Accepted => {}
        Receipt::Retry { error, .. } => warn!(
            "[email-outbox] {} email {} attempt {} will be retried: {error}",
            claimed.kind, claimed.id, claimed.attempts
        ),
        Receipt::Rejected(error) => error!(
            "[email-outbox] {} email {} was rejected and will not be sent: {error}",
            claimed.kind, claimed.id
        ),
    }
}

/// Writes `transition` for a held row; false when it could not be recorded.
async fn record(db: &DatabaseConnection, claimed: &Claimed, transition: Transition) -> bool {
    let now = Utc::now().naive_utc();
    let written = match &transition {
        Transition::Sent => email_outbox::mark_sent(db, claimed, now).await,
        Transition::Retry { not_before, error } => {
            email_outbox::release(db, claimed, now, *not_before, error).await
        }
        Transition::Fail(error) => email_outbox::fail(db, claimed, now, error).await,
    };
    match written {
        Ok(true) => true,
        Ok(false) => {
            warn!(
                "[email-outbox] {} email {} lost its claim before {transition:?} was recorded",
                claimed.kind, claimed.id
            );
            false
        }
        Err(e) => {
            warn!(
                "[email-outbox] could not record {transition:?} for {} email {}; its lease \
                 will lapse and it will be tried again: {e:?}",
                claimed.kind, claimed.id
            );
            false
        }
    }
}

/// Deletes outbox rows finished more than [`RETENTION`] ago.
#[derive(Default)]
pub struct Purge;

#[async_trait]
impl Job for Purge {
    fn name(&self) -> &'static str {
        "email-outbox-purge"
    }

    fn interval(&self) -> Duration {
        PURGE_INTERVAL
    }

    async fn run(&self, ctx: &Context) -> Result<Outcome, Error> {
        email_outbox::purge(&*ctx.db, Utc::now().naive_utc() - RETENTION)
            .await
            .map(Outcome::processed)
            .map_err(Into::into)
    }
}

#[cfg(test)]
#[path = "email_outbox_sqlite_tests.rs"]
mod sqlite_tests;
