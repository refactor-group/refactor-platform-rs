# Email outbox

Every email the backend sends is written as a row in `refactor_platform.email_outbox`, in
the same database transaction as the change it announces. A row holds a fully rendered,
provider-neutral email (`OutgoingEmail`: from, to, reply-to, subject, HTML, plain text,
attachments), never a provider's template request. One background sweep is the only code
that hands outbox email to a provider. This document describes that system and the
guarantees it makes. Senders move onto it as their templates are rendered in-repo; until
a sender has moved, it still sends through a Resend-hosted template.

## Guarantees

| Guarantee | How |
|---|---|
| **Never lost.** A committed change always owes its email; a rolled-back change owes none. | The row is inserted on the caller's transaction. |
| **Never sent twice.** | Each row's id is its Resend `Idempotency-Key` (`email-outbox/{id}`). Every attempt sends the same stored body under the same key, so Resend delivers it once. |
| **Survives crashes and deploys.** | Rows are durable. A claim is a lease; a row whose sender died is claimable again when the lease lapses. The key makes the repeat safe. |
| **Safe with many replicas.** | Rows are claimed with `FOR UPDATE SKIP LOCKED` and every state change is conditional on the caller still holding the claim. |
| **Ordered per conversation.** | A row is not claimed while an older pending row exists for the same subject and recipient. Writers to one conversation are serialized by a transaction-scoped advisory lock taken before the rows are stamped, so rows are ordered by commit, assuming replica clocks are NTP-synced. |
| **Never outlives the key window.** | Resend remembers a key for 24 hours. A row older than 20 hours is failed without an attempt, so a late retry can never become a duplicate. |
| **A render error never undoes the user's action.** | Senders render before enqueueing and log and skip a render or configuration failure. Only the insert itself can fail the caller's transaction. |
| **Requests never wait on Resend.** | Requests only insert rows. |

Delivery is attempted at least once and delivered at most once within the key window.
One edge: a row Resend accepted whose "sent" write then failed is retried; if that retry
falls after the 20-hour cutoff it is recorded as failed although it was delivered.

## Why an outbox, given "sweeps, not a queue"

[`background_jobs.md`](background_jobs.md) rejects a durable queue because a queued job is
told at enqueue time what work exists and goes stale when state changes. That applies to
work derived from state, such as a reminder ("the session starts in 24 hours"). It does
not apply to an email that records an event ("this session was scheduled for 15:00"):
the event stays true after later edits, and a later edit writes its own email. So:

- Derived work stays a sweep over current rows (reminders, reschedule settling, owed
  invites, purges). Such a sweep decides an email is owed and writes an outbox row.
- The outbox carries only event emails, rendered when written and sent verbatim.

## Why not a crate

Evaluated: apalis, graphile_worker, pgmq, underway, sqlxmq, fang, obix, outbox-postgres.

- SeaORM 2's `DatabaseTransaction` exposes no sqlx transaction, so no crate's typed
  enqueue API can join our transaction; every one would need raw SQL against its schema.
- Most still depend on sqlx 0.8, which would need a second sqlx and a second pool.
- The rest bring a fixed schema of their own, migrations run at worker startup outside
  `migrationctl`, session advisory locks, `LISTEN`, or an extension DigitalOcean does not
  offer, each of which conflicts with the PgBouncer transaction-mode pool or with our
  migration rules.

What an email outbox needs is small: an insert, a `SKIP LOCKED` claim with a lease,
backoff, a terminal failed state and an idempotency key. It follows patterns this
codebase already uses (`coaching_session_pending_reschedule`).

## Data model

`refactor_platform.email_outbox`:

| Column | Meaning |
|---|---|
| `id` uuid | Row id; the Resend idempotency key is derived from it. |
| `kind` text | Stable notification name, for logs and audit. |
| `subject_key` text | Ordering scope: `coaching_relationship/{id}` for every session and series email, `user/{id}` otherwise. |
| `recipient_user_id` uuid | Addressee; cascades on user delete. |
| `position` smallint | Order within the batch that wrote it. |
| `request` jsonb null | The rendered `OutgoingEmail`, sent as stored on every attempt. Cleared when the row finishes. |
| `state` text | `pending`, `sent` or `failed` (CHECK constraint, not a Postgres enum). |
| `attempts` smallint | Attempts so far. |
| `not_before` timestamp | Earliest next attempt. |
| `claim_id` uuid null, `lease_until` timestamp null | The current claim and when it lapses. |
| `last_error` text null | Last failure as Resend described it. Never the body. |
| `created_at`, `updated_at`, `finished_at` | `finished_at` is set when the row becomes `sent` or `failed`. |

Pending rows are ordered by `(created_at, position, id)`. Partial indexes on pending rows
serve the claim and the ordering check; a partial index on finished rows' `finished_at`
serves the purge.

### Privacy

Bodies hold addresses, names, meeting links, session details and, for welcome and
password-reset emails, sign-in links. So a body is cleared the moment its row finishes,
finished rows are deleted after 7 days, and bodies are never logged.

## Lifecycle

```
pending ──claim──▶ claimed ──Accepted──▶ sent
   ▲                  │  ├──Rejected──▶ failed
   └──Retry(backoff)──┘  └──older than 20h──▶ failed (no attempt)
```

- **Claim**, one statement: pending rows whose `not_before` has passed, whose lease is free
  or lapsed, and with no older pending row for the same subject and recipient; oldest
  first, at most a batch, `FOR UPDATE SKIP LOCKED`. Sets the claim and the lease and
  counts the attempt.
- **Accepted**: Resend's 2xx, or its 409 `invalid_idempotent_request`, which means it
  already sent an email under this key.
- **Retry**: timeouts, connection errors, 429, 5xx, and the 409
  `concurrent_idempotent_requests`. The next attempt waits for the backoff or Resend's
  `Retry-After`, whichever is longer.
- **Rejected**: any other 4xx. Logged at ERROR. A failed row no longer blocks later rows.

Constants, not configuration: tick 1s, batch 10, concurrency 5, lease 5 minutes, backoff
30s doubling to a 30-minute cap, cutoff 20 hours, retention 7 days. The sweep polls
every second instead of waking on commit: one indexed query on an empty partial index is
cheaper than threading a wake handle through every sender, and `LISTEN`/`NOTIFY` does not
work through the transaction-mode connection pool. A row whose stored email no longer
parses is failed with "stored email could not be read", never sent.

## Providers

The provider sits behind the `Mailer` trait, which takes an `OutgoingEmail` and returns a
provider-neutral `Receipt`. `domain::gateway::mailer::from_config` is the one place that
chooses the provider; today that is Resend, whose `Mailer` maps the email onto its send
fields. In production the server refuses to start when no provider is configured or it
fails to build. Elsewhere the sweep warns once at startup and fails each due row with
"email is not configured", so pending bodies never pile up.

## Code layout

| Layer | What |
|---|---|
| `entity::email_outbox` | The row. |
| `entity_api::email_outbox` | `insert`, `claim`, `mark_sent`, `release`, `fail`, `purge`: one statement each, every transition filtered by the held claim. |
| `domain::email_outbox` | `OutgoingEmail`, `Envelope` (the email with its subject and recipient), `enqueue`, the `Mailer` trait and its `Receipt`, `RetryPolicy` and the pure `settle`, which maps a receipt to the row's next state. |
| `domain::gateway::mailer` | `from_config`: the configured `Mailer`, if any. |
| `domain::gateway::resend` | `Mailer` for Resend: maps an `OutgoingEmail` onto its send fields and classifies responses into `Receipt`. |
| `domain::jobs::email_outbox` | The sweep and the purge, composed from a `Mailer` and a `RetryPolicy`. |

## Phases

1. This infrastructure, with no sender moved. No behavior change.
2. Senders move one at a time as their templates are rendered in-repo on the shared
   layout (issue #525, [Email composition](email_composition.md)). Each sender's move
   ports its template, renders and enqueues in the transaction it announces, and retires
   its `*_TEMPLATE_ID`. Sweeps that decide an email is owed (reminders, pending
   reschedules, owed invites) claim, render and enqueue in one transaction. The
   single-session cancellation, session scheduled and session rescheduled emails, and the
   session invites the pending reschedule sweep owes, have moved. A session invite resent on request is
   composed the same way but handed to the `Mailer` directly, since its endpoint still
   reports whether it was delivered.
3. Once every sender has moved, `gateway::resend::Client::send_email` and the template
   path are removed, and the outbox is the only way email leaves.
