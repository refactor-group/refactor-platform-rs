//! Emails each session's net reschedule once its schedule has settled.
//!
//! A session edit records a pending reschedule instead of emailing. This sweep claims the
//! rows that have come due and sends the session-rescheduled email, moved from the start
//! participants were last told. A row is due once the schedule has been quiet for the
//! settle period, the first unannounced change is older than the cap, or the session
//! starts within the cap.
//!
//! Rows are claimed under a lease, so delivery is at least once: a send that outlives
//! [`coaching_session_pending_reschedule::LEASE`], or a crash between a send and its completion, can repeat.
//! A failed send is retried up to [`coaching_session_pending_reschedule::MAX_ATTEMPTS`] times, then given up;
//! the regular reminder is the fallback.
//!
//! Only participants still in the session's organization are emailed. The change settles
//! once everyone but its schedule owner was told or has left the organization, since someone
//! who left is owed nothing.
//!
//! Without Resend or the rescheduled template the sweep still runs, dropping due rows
//! unsent, since a pending row suppresses the session's reminder.
//!
//! A delivered send leaves the row to `coaching_session_pending_reschedule::complete`, which
//! keeps it when a newer change landed mid-send. Each participant whose copy went out has
//! their own notice restamped, even when another's failed and the send is retried.
//!
//! The same sweep re-sends the invites owed to participants after their relationship changed
//! kind (`coaching_session_pending_invite`): one email per row, to that participant alone,
//! under the same lease, retry and give-up rules. A participant who has left the
//! organization, a session that has passed and a series with nothing left to attend are
//! owed nothing. A session's invite is written to the email outbox in one transaction with
//! its `SEQUENCE` bump and the row's completion; a failed load or write releases the row to
//! retry, and a row is dropped unsent when no email can be composed. A series' invite is
//! sent through its Resend template, and a row for one is dropped unsent without that
//! template or a Resend key. A switch can owe many invites at once, so a tick claims at most
//! [`INVITE_BATCH`] of them, and a failed series send hands the rest of the tick's back
//! uncharged, as the reminder sweep does when Resend is unusable: the interval is the
//! backoff.
//!
//! See [`crate::jobs`] for why this is a sweep rather than an enqueued job.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use entity_api::coaching_session;
use entity_api::coaching_session_pending_invite::{self, Target};
use entity_api::coaching_session_pending_reschedule::{
    self, Claimed, Completed, Released, Schedule, Timing,
};
use entity_api::error::EntityApiErrorKind;
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::coaching_session::record_delivered_notice;
use crate::coaching_session_series;
use crate::emails::{self, Delivered, Delivery};
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::jobs::{Context, FirstRun, Job, Outcome};
use crate::{coaching_relationships, coaching_sessions, Id};

/// How often the sweep looks for due rows.
const INTERVAL: Duration = Duration::from_secs(15);

/// Most pending reschedules one tick will claim.
const BATCH: u64 = 50;

/// Most owed invites one tick will claim, so a large switch drains over several ticks
/// rather than bursting Resend.
const INVITE_BATCH: u64 = 10;

/// Settle period used when the configured one does not fit a `TimeDelta`.
const DEFAULT_SETTLE: TimeDelta = TimeDelta::seconds(60);

/// Cap used when the configured one does not fit a `TimeDelta`.
const DEFAULT_SETTLE_MAX: TimeDelta = TimeDelta::seconds(300);

pub struct Sweep {
    timing: Timing,
    mode: Mode,
    /// Whether a series' owed invite is re-sent or dropped unsent.
    series_invites: Mode,
}

/// Whether due rows are emailed or dropped unsent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Send,
    Drain,
}

impl Mode {
    /// Sends when Resend has a key and the template is set; otherwise drains, saying `emails` are off.
    fn of(config: &Config, template: Option<&str>, emails: &str) -> Self {
        if config.resend_api_key().is_some() && template.is_some() {
            Mode::Send
        } else {
            info!("[coaching-session-pending-reschedule] {emails} are off; those owed will be dropped unsent");
            Mode::Drain
        }
    }
}

/// What one owed invite's turn left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resent {
    Handled,
    Unhandled,
    /// Resend did not take the email, so the rest of the tick waits for the next one.
    SendFailed,
}

impl From<bool> for Resent {
    fn from(handled: bool) -> Self {
        match handled {
            true => Resent::Handled,
            false => Resent::Unhandled,
        }
    }
}

impl Sweep {
    /// Builds the sweep from config, draining unsent when reschedule emails are off.
    pub fn from_config(config: &Config) -> Self {
        let timing = Timing {
            settle: delta_or(
                config.session_reschedule_settle(),
                DEFAULT_SETTLE,
                "SESSION_RESCHEDULE_SETTLE_SECONDS",
            ),
            settle_max: delta_or(
                config.session_reschedule_settle_max(),
                DEFAULT_SETTLE_MAX,
                "SESSION_RESCHEDULE_SETTLE_MAX_SECONDS",
            ),
            lease: coaching_session_pending_reschedule::LEASE,
        };

        Self {
            timing,
            mode: Mode::of(
                config,
                config.session_rescheduled_email_template_id().as_deref(),
                "reschedule emails",
            ),
            series_invites: Mode::of(
                config,
                config
                    .recurring_sessions_scheduled_email_template_id()
                    .as_deref(),
                "series invite emails",
            ),
        }
    }

    /// Re-sends, writes or drops one claimed owed invite.
    async fn resend_invite(
        &self,
        ctx: &Context,
        claimed: &coaching_session_pending_invite::Claimed,
    ) -> Resent {
        match claimed.target {
            Target::Session(id) => enqueue_session_invite(ctx, id, claimed).await,
            Target::Series(id) => self.resend_series_invite(ctx, id, claimed).await,
        }
    }

    /// Re-sends or drops one claimed owed series invite.
    async fn resend_series_invite(
        &self,
        ctx: &Context,
        id: Id,
        claimed: &coaching_session_pending_invite::Claimed,
    ) -> Resent {
        let db = &*ctx.db;
        let user_id = claimed.user_id;
        let resent = match self.series_invites {
            Mode::Drain => Ok(None),
            Mode::Send => {
                coaching_session_series::resend_invite_to(db, &ctx.config, id, user_id).await
            }
        };
        match resent {
            Ok(Some(Delivery::Failed)) => {
                warn!(
                    "[coaching-session-pending-reschedule] invite {:?} did not reach {user_id}",
                    claimed.target
                );
                retry_invite(db, claimed).await;
                Resent::SendFailed
            }
            Ok(_) => Resent::from(complete_invite(db, claimed).await),
            Err(e) => {
                warn!("[coaching-session-pending-reschedule] could not resend invite {:?} to {user_id}: {e:?}", claimed.target);
                Resent::from(retry_invite(db, claimed).await)
            }
        }
    }

    /// Emails or drops one claimed row; returns whether it was handled.
    async fn handle(&self, ctx: &Context, claimed: &Claimed) -> bool {
        let db = &*ctx.db;
        let id = claimed.coaching_session_id;
        let session = match coaching_session::find_by_id(db, id).await {
            Ok(session) => session,
            // The session's deletion cascaded to the row.
            Err(e) if e.error_kind == EntityApiErrorKind::RecordNotFound => return true,
            Err(e) => {
                warn!("[coaching-session-pending-reschedule] could not load session {id}: {e:?}");
                return false;
            }
        };

        let current = Schedule {
            date: session.date,
            duration_minutes: session.duration_minutes,
        };
        if self.mode == Mode::Drain || current == claimed.announced {
            return settle(db, claimed, current).await;
        }

        match emails::send_session_rescheduled(db, &ctx.config, &session, claimed.announced.date)
            .await
        {
            Ok((relationship, delivered)) => {
                record_participant_notice(db, &session, &relationship, &delivered).await;
                match delivered.settles(&relationship, session.scheduled_by_user_id) {
                    true => settle(db, claimed, current).await,
                    false => {
                        warn!("[coaching-session-pending-reschedule] session {id} was not told to everyone owed it: {delivered:?}");
                        retry(db, claimed).await
                    }
                }
            }
            Err(e) => {
                warn!("[coaching-session-pending-reschedule] send failed for session {id}: {e:?}");
                retry(db, claimed).await
            }
        }
    }
}

/// Writes one claimed owed session invite and settles its row together. A stack that composes
/// no email settles the row unsent; any other failure releases it to retry.
async fn enqueue_session_invite(
    ctx: &Context,
    id: Id,
    claimed: &coaching_session_pending_invite::Claimed,
) -> Resent {
    let db = &*ctx.db;
    match crate::coaching_session::enqueue_owed_invite(db, &ctx.config, id, claimed).await {
        Ok(()) => Resent::Handled,
        Err(e)
            if matches!(
                e.error_kind,
                DomainErrorKind::Internal(InternalErrorKind::Config)
            ) =>
        {
            warn!(
                "[coaching-session-pending-reschedule] session {id}'s invite to {} is dropped unsent: no email can be composed: {e:?}",
                claimed.user_id
            );
            Resent::from(complete_invite(db, claimed).await)
        }
        Err(e) => {
            warn!(
                "[coaching-session-pending-reschedule] could not write session {id}'s invite to {}: {e:?}",
                claimed.user_id
            );
            Resent::from(retry_invite(db, claimed).await)
        }
    }
}

/// Restamps the notice of each participant `delivered` reached. Best-effort: a failure is
/// logged.
async fn record_participant_notice(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    relationship: &coaching_relationships::Model,
    delivered: &Delivered,
) {
    if ![delivered.coachee, delivered.coach].contains(&Delivery::Sent) {
        return;
    }
    record_delivered_notice(db, relationship, delivered, &[session.id]).await
}

/// Converts a configured duration, falling back to `default` when it overflows.
fn delta_or(duration: Duration, default: TimeDelta, name: &str) -> TimeDelta {
    TimeDelta::from_std(duration).unwrap_or_else(|e| {
        warn!(
            "[coaching-session-pending-reschedule] {name} is out of range, using {default}: {e:?}"
        );
        default
    })
}

/// Completes a claimed row after `sent` was told or nothing needed telling.
async fn settle(db: &DatabaseConnection, claimed: &Claimed, sent: Schedule) -> bool {
    let id = claimed.coaching_session_id;
    match coaching_session_pending_reschedule::complete(db, claimed, sent).await {
        Ok(Completed::Settled | Completed::StillPending) => true,
        Ok(Completed::Superseded) => {
            info!("[coaching-session-pending-reschedule] another claim took over session {id}");
            true
        }
        Err(e) => {
            warn!("[coaching-session-pending-reschedule] could not complete session {id}: {e:?}");
            false
        }
    }
}

/// Releases a claimed row after a failed send; returns whether it was handled.
async fn retry(db: &DatabaseConnection, claimed: &Claimed) -> bool {
    let id = claimed.coaching_session_id;
    match coaching_session_pending_reschedule::release(db, claimed).await {
        Ok(Released::Retrying) => false,
        Ok(Released::GaveUp) => {
            warn!(
                "[coaching-session-pending-reschedule] giving up on session {id}; the change will not be \
                 emailed and the reminder is the fallback"
            );
            false
        }
        Ok(Released::Superseded) => {
            info!("[coaching-session-pending-reschedule] another claim took over session {id}");
            true
        }
        Err(e) => {
            warn!("[coaching-session-pending-reschedule] could not release session {id}: {e:?}");
            false
        }
    }
}

/// Drops a claimed owed invite once sent or no longer owed; returns whether it was handled.
async fn complete_invite(
    db: &DatabaseConnection,
    claimed: &coaching_session_pending_invite::Claimed,
) -> bool {
    coaching_session_pending_invite::complete(db, claimed)
        .await
        .inspect_err(|e| {
            warn!(
                "[coaching-session-pending-reschedule] could not complete invite {}: {e:?}",
                claimed.id
            )
        })
        .is_ok()
}

/// Hands back a claimed owed invite the tick did not attempt. Best-effort: its lease runs out
/// anyway.
async fn hand_back_invite(
    db: &DatabaseConnection,
    claimed: &coaching_session_pending_invite::Claimed,
) {
    let _ = coaching_session_pending_invite::hand_back(db, claimed)
        .await
        .inspect_err(|e| {
            warn!(
                "[coaching-session-pending-reschedule] could not hand back invite {}: {e:?}",
                claimed.id
            )
        });
}

/// Releases a claimed owed invite after a failed send; returns whether it was handled.
async fn retry_invite(
    db: &DatabaseConnection,
    claimed: &coaching_session_pending_invite::Claimed,
) -> bool {
    let id = claimed.id;
    match coaching_session_pending_invite::release(db, claimed).await {
        Ok(Released::Retrying) => false,
        Ok(Released::GaveUp) => {
            warn!("[coaching-session-pending-reschedule] giving up on invite {id}");
            false
        }
        Ok(Released::Superseded) => true,
        Err(e) => {
            warn!("[coaching-session-pending-reschedule] could not release invite {id}: {e:?}");
            false
        }
    }
}

#[async_trait]
impl Job for Sweep {
    fn name(&self) -> &'static str {
        "coaching-session-pending-reschedule"
    }

    fn interval(&self) -> Duration {
        INTERVAL
    }

    fn first_run(&self) -> FirstRun {
        FirstRun::Immediately
    }

    async fn run(&self, ctx: &Context) -> Result<Outcome, Error> {
        let now = Utc::now().naive_utc();
        let claimed = coaching_session_pending_reschedule::claim_due(
            &*ctx.db,
            now,
            self.timing,
            Id::new_v4(),
            BATCH,
        )
        .await?;
        let owed =
            coaching_session_pending_invite::claim(&*ctx.db, now, Id::new_v4(), INVITE_BATCH)
                .await?;

        let mut handled = 0;
        for row in &claimed {
            if self.handle(ctx, row).await {
                handled += 1;
            }
        }
        let mut unattempted = owed.iter();
        for row in unattempted.by_ref() {
            match self.resend_invite(ctx, row).await {
                Resent::Handled => handled += 1,
                Resent::Unhandled => {}
                Resent::SendFailed => break,
            }
        }
        for row in unattempted {
            hand_back_invite(&ctx.db, row).await;
        }

        Ok(Outcome::partial(
            handled,
            (claimed.len() + owed.len()) as u64,
        ))
    }
}

#[cfg(test)]
#[path = "coaching_session_pending_reschedule_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn config(args: &[&str]) -> Config {
        let mut argv = vec!["test"];
        argv.extend_from_slice(args);
        Config::from_args(argv)
    }

    #[test]
    fn from_config_drains_without_a_resend_key() {
        let sweep = Sweep::from_config(&config(&[
            "--session-rescheduled-email-template-id=rescheduled_tpl",
        ]));

        assert_eq!(sweep.mode, Mode::Drain);
    }

    #[test]
    fn from_config_drains_without_a_template() {
        let sweep = Sweep::from_config(&config(&["--resend-api-key=key"]));

        assert_eq!(sweep.mode, Mode::Drain);
    }

    #[test]
    fn from_config_sends_with_a_key_and_a_template() {
        let sweep = Sweep::from_config(&config(&[
            "--resend-api-key=key",
            "--session-rescheduled-email-template-id=rescheduled_tpl",
        ]));

        assert_eq!(sweep.mode, Mode::Send);
    }

    #[test]
    fn from_config_resends_series_invites_with_a_key_and_their_template() {
        let mode = |args: &[&str]| Sweep::from_config(&config(args)).series_invites;
        let series = "--recurring-sessions-scheduled-email-template-id=recurring_tpl";
        let key = "--resend-api-key=key";

        assert_eq!(mode(&[key, series]), Mode::Send);
        assert_eq!(mode(&[key]), Mode::Drain);
        assert_eq!(mode(&[series]), Mode::Drain);
    }

    #[test]
    fn from_config_reads_timing_from_config() {
        let sweep = Sweep::from_config(&config(&[
            "--session-reschedule-settle-seconds=90",
            "--session-reschedule-settle-max-seconds=600",
        ]));

        assert_eq!(sweep.timing.settle, TimeDelta::seconds(90));
        assert_eq!(sweep.timing.settle_max, TimeDelta::seconds(600));
        assert_eq!(
            sweep.timing.lease,
            coaching_session_pending_reschedule::LEASE
        );
    }
}
