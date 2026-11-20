//! Emails written in the transaction they announce and handed to the provider by one sweep.
//!
//! A sender renders its email into an [`OutgoingEmail`], wraps it in an [`Envelope`] and
//! [`enqueue`]s it on the caller's
//! transaction, so a committed change always owes its email and a rolled-back one owes
//! none. The sweep in [`crate::jobs::email_outbox`] delivers each row through a [`Mailer`]
//! under the row's own idempotency key, and [`settle`] decides what the [`Receipt`] leaves
//! behind. See `docs/architecture/email_outbox.md`.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{NaiveDateTime, TimeDelta, Utc};
use entity_api::email_outbox::{self, New};
use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::Id;

/// An address, with the name shown beside it when there is one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub email: String,
    pub name: Option<String>,
}

/// A file sent with an email.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    /// The file's bytes, base64-encoded.
    pub content: String,
}

/// A fully rendered email, ready for any provider.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingEmail {
    pub from: Mailbox,
    pub to: Vec<Mailbox>,
    /// Where replies go, when not to `from`.
    pub reply_to: Option<Mailbox>,
    pub subject: String,
    pub html: String,
    /// The plain-text alternative to `html`.
    pub text: String,
    pub attachments: Vec<Attachment>,
}

/// Shows who it is between and what it carries, never its words, links or file contents.
impl fmt::Debug for OutgoingEmail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let to = self
            .to
            .iter()
            .map(|mailbox| mailbox.email.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let reply_to = self.reply_to.as_ref().map_or(String::new(), |mailbox| {
            format!(", replies to {}", mailbox.email)
        });
        let recipients = format!("from {} to {to}{reply_to}", self.from.email);
        let attachments: Vec<&str> = self
            .attachments
            .iter()
            .map(|attachment| attachment.filename.as_str())
            .collect();
        f.debug_struct("OutgoingEmail")
            .field("recipients", &recipients)
            .field("attachments", &attachments)
            .finish_non_exhaustive()
    }
}

/// The conversation an email is ordered within, per recipient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    /// Every session and series email in one coaching relationship.
    CoachingRelationship(Id),
    /// Every other email about one user.
    User(Id),
}

impl Subject {
    /// The stored form of this subject.
    pub fn key(&self) -> String {
        match self {
            Subject::CoachingRelationship(id) => format!("coaching_relationship/{id}"),
            Subject::User(id) => format!("user/{id}"),
        }
    }
}

/// A rendered email with the conversation it belongs to and who receives it.
#[derive(Debug)]
pub struct Envelope {
    /// Stable notification name, for logs and audit.
    pub kind: &'static str,
    pub subject: Subject,
    pub recipient_id: Id,
    pub email: OutgoingEmail,
}

impl TryFrom<Envelope> for New {
    type Error = Error;

    fn try_from(envelope: Envelope) -> Result<Self, Error> {
        Ok(New {
            kind: envelope.kind.to_string(),
            subject_key: envelope.subject.key(),
            recipient_user_id: envelope.recipient_id,
            request: serde_json::to_value(&envelope.email)?,
        })
    }
}

/// Writes `envelopes` to the outbox on `db`, in order; returns how many were written.
///
/// Pass the transaction that makes the change the emails announce. Each conversation is
/// locked before the rows are stamped, so a second writer to it waits for this one to
/// commit and stamps a later time.
pub async fn enqueue(
    db: &impl ConnectionTrait,
    envelopes: impl IntoIterator<Item = Envelope>,
) -> Result<usize, Error> {
    let rows = envelopes
        .into_iter()
        .map(New::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let count = rows.len();
    email_outbox::lock_conversations(db, &rows).await?;
    email_outbox::insert(db, rows, Utc::now().naive_utc()).await?;
    Ok(count)
}

/// What the provider made of one delivery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Receipt {
    /// Sent now, or already sent under this key.
    Accepted,
    /// Came at a bad time; the same request may succeed later, no sooner than `after`.
    Retry {
        after: Option<Duration>,
        error: String,
    },
    /// The request itself was refused, so sending it again gets the same answer.
    Rejected(String),
}

/// Hands one rendered email to an email provider.
#[async_trait]
pub trait Mailer: Send + Sync + 'static {
    /// Delivers `email` under idempotency `key`, so a repeat with the same key sends once.
    async fn deliver(&self, key: &str, email: &OutgoingEmail) -> Receipt;
}

/// The idempotency key of the outbox row `id`.
pub fn idempotency_key(id: Id) -> String {
    format!("email-outbox/{id}")
}

/// What a row becomes after an attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transition {
    Sent,
    Retry {
        not_before: NaiveDateTime,
        error: String,
    },
    Fail(String),
}

/// How retries back off, when a row is too old to try, and how long a claim lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Wait after the first failed attempt; doubles with each one after.
    pub base: TimeDelta,
    /// Longest wait between attempts.
    pub cap: TimeDelta,
    /// Age past which a row is failed without an attempt, inside the provider's key window.
    pub max_age: TimeDelta,
    /// How long a sweep's claim holds a row.
    pub lease: TimeDelta,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base: TimeDelta::seconds(30),
            cap: TimeDelta::minutes(30),
            max_age: TimeDelta::hours(20),
            lease: TimeDelta::minutes(5),
        }
    }
}

impl RetryPolicy {
    /// Wait after `attempts` failed attempts, or the provider's `retry_after` when longer.
    pub fn backoff(&self, attempts: i16, retry_after: Option<Duration>) -> TimeDelta {
        let doublings = u32::try_from(attempts.saturating_sub(1)).unwrap_or(0);
        let exponential = 2i32
            .checked_pow(doublings)
            .and_then(|factor| self.base.checked_mul(factor))
            .map_or(self.cap, |wait| wait.min(self.cap));
        // Any wait past `max_age` ends in expiry, so it is clamped there.
        let requested = retry_after.map_or(TimeDelta::zero(), |after| {
            TimeDelta::from_std(after).map_or(self.max_age, |after| after.min(self.max_age))
        });
        exponential.max(requested)
    }

    /// Whether a row written at `created_at` is too old to attempt at `now`.
    pub fn is_expired(&self, created_at: NaiveDateTime, now: NaiveDateTime) -> bool {
        now - created_at >= self.max_age
    }
}

/// What `receipt` for attempt number `attempts` at `now` leaves the row as.
pub fn settle(
    receipt: &Receipt,
    attempts: i16,
    now: NaiveDateTime,
    policy: &RetryPolicy,
) -> Transition {
    match receipt {
        Receipt::Accepted => Transition::Sent,
        Receipt::Retry { after, error } => Transition::Retry {
            not_before: now + policy.backoff(attempts, *after),
            error: error.clone(),
        },
        Receipt::Rejected(error) => Transition::Fail(error.clone()),
    }
}

#[cfg(test)]
#[path = "email_outbox_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "email_outbox_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(all(test, feature = "mock"))]
#[path = "email_outbox_mock_tests.rs"]
mod mock_tests;
