//! Emails each session's net reschedule once its schedule has settled.
//!
//! A session edit records a pending reschedule instead of emailing. This sweep claims the
//! rows that have come due and sends the session-rescheduled email, moved from the start
//! participants were last told. A row is due once the schedule has been quiet for the
//! settle period, the first unannounced change is older than the cap, or the session
//! starts within the cap.
//!
//! Rows are claimed under a lease, so delivery is at least once: a send that outlives
//! [`pending_reschedule::LEASE`], or a crash between a send and its completion, can repeat.
//! A failed send is retried up to [`pending_reschedule::MAX_ATTEMPTS`] times, then given up;
//! the regular reminder is the fallback.
//!
//! Without Resend or the rescheduled template the sweep still runs, dropping due rows
//! unsent, since a pending row suppresses the session's reminder.
//!
//! A delivered send stamps notice with `coaching_session::mark_notice_given` alone, leaving
//! the row to `pending_reschedule::complete`, which keeps it when a newer change landed
//! mid-send.
//!
//! See [`crate::jobs`] for why this is a sweep rather than an enqueued job.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use entity_api::coaching_session;
use entity_api::error::EntityApiErrorKind;
use entity_api::pending_reschedule::{self, Claimed, Completed, Released, Schedule, Timing};
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::emails;
use crate::error::Error;
use crate::jobs::{Context, FirstRun, Job, Outcome};
use crate::Id;

/// How often the sweep looks for due rows.
const INTERVAL: Duration = Duration::from_secs(15);

/// Most rows one tick will claim.
const BATCH: u64 = 50;

/// Settle period used when the configured one does not fit a `TimeDelta`.
const DEFAULT_SETTLE: TimeDelta = TimeDelta::seconds(60);

/// Cap used when the configured one does not fit a `TimeDelta`.
const DEFAULT_CAP: TimeDelta = TimeDelta::seconds(300);

pub struct Sweep {
    timing: Timing,
    mode: Mode,
}

/// Whether due rows are emailed or dropped unsent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Send,
    Drain,
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
            cap: delta_or(
                config.session_reschedule_settle_max(),
                DEFAULT_CAP,
                "SESSION_RESCHEDULE_SETTLE_MAX_SECONDS",
            ),
            lease: pending_reschedule::LEASE,
        };

        let sendable = config.resend_api_key().is_some()
            && config.session_rescheduled_email_template_id().is_some();
        let mode = if sendable {
            Mode::Send
        } else {
            info!(
                "[pending-reschedule] reschedule emails are off; pending changes will be \
                 dropped unsent"
            );
            Mode::Drain
        };

        Self { timing, mode }
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
                warn!("[pending-reschedule] could not load session {id}: {e:?}");
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
            Ok(delivered) if delivered.coachee => {
                let handled = settle(db, claimed, current).await;
                if let Err(e) = coaching_session::mark_notice_given(db, id).await {
                    warn!("[pending-reschedule] could not record notice for session {id}: {e:?}");
                }
                handled
            }
            Ok(delivered) => {
                warn!("[pending-reschedule] session {id} coachee was not told: {delivered:?}");
                retry(db, claimed).await
            }
            Err(e) => {
                warn!("[pending-reschedule] send failed for session {id}: {e:?}");
                retry(db, claimed).await
            }
        }
    }
}

/// Converts a configured duration, falling back to `default` when it overflows.
fn delta_or(duration: Duration, default: TimeDelta, name: &str) -> TimeDelta {
    TimeDelta::from_std(duration).unwrap_or_else(|e| {
        warn!("[pending-reschedule] {name} is out of range, using {default}: {e:?}");
        default
    })
}

/// Completes a claimed row after `sent` was told or nothing needed telling.
async fn settle(db: &DatabaseConnection, claimed: &Claimed, sent: Schedule) -> bool {
    let id = claimed.coaching_session_id;
    match pending_reschedule::complete(db, claimed, sent).await {
        Ok(Completed::Settled | Completed::StillPending) => true,
        Ok(Completed::Superseded) => {
            info!("[pending-reschedule] another claim took over session {id}");
            true
        }
        Err(e) => {
            warn!("[pending-reschedule] could not complete session {id}: {e:?}");
            false
        }
    }
}

/// Releases a claimed row after a failed send; returns whether it was handled.
async fn retry(db: &DatabaseConnection, claimed: &Claimed) -> bool {
    let id = claimed.coaching_session_id;
    match pending_reschedule::release(db, claimed).await {
        Ok(Released::Retrying) => false,
        Ok(Released::GaveUp) => {
            warn!(
                "[pending-reschedule] giving up on session {id}; the change will not be \
                 emailed and the reminder is the fallback"
            );
            false
        }
        Ok(Released::Superseded) => {
            info!("[pending-reschedule] another claim took over session {id}");
            true
        }
        Err(e) => {
            warn!("[pending-reschedule] could not release session {id}: {e:?}");
            false
        }
    }
}

#[async_trait]
impl Job for Sweep {
    fn name(&self) -> &'static str {
        "pending-reschedule"
    }

    fn interval(&self) -> Duration {
        INTERVAL
    }

    fn first_run(&self) -> FirstRun {
        FirstRun::Immediately
    }

    async fn run(&self, ctx: &Context) -> Result<Outcome, Error> {
        let claimed = pending_reschedule::claim_due(
            &*ctx.db,
            Utc::now().naive_utc(),
            self.timing,
            Id::new_v4(),
            BATCH,
        )
        .await?;

        if claimed.is_empty() {
            return Ok(Outcome::IDLE);
        }

        let mut handled = 0;
        for row in &claimed {
            if self.handle(ctx, row).await {
                handled += 1;
            }
        }

        Ok(Outcome::partial(handled, claimed.len() as u64))
    }
}

#[cfg(test)]
#[path = "pending_reschedule_sqlite_tests.rs"]
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
    fn from_config_reads_timing_from_config() {
        let sweep = Sweep::from_config(&config(&[
            "--session-reschedule-settle-seconds=90",
            "--session-reschedule-settle-max-seconds=600",
        ]));

        assert_eq!(sweep.timing.settle, TimeDelta::seconds(90));
        assert_eq!(sweep.timing.cap, TimeDelta::seconds(600));
        assert_eq!(sweep.timing.lease, pending_reschedule::LEASE);
    }
}
