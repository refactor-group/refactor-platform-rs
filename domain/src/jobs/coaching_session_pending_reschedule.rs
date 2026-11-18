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
//! owed nothing. Without Resend or the scheduled templates those rows are dropped unsent.
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
use crate::error::Error;
use crate::jobs::{Context, FirstRun, Job, Outcome};
use crate::{coaching_relationships, coaching_sessions, Id};

/// How often the sweep looks for due rows.
const INTERVAL: Duration = Duration::from_secs(15);

/// Most rows one tick will claim.
const BATCH: u64 = 50;

/// Settle period used when the configured one does not fit a `TimeDelta`.
const DEFAULT_SETTLE: TimeDelta = TimeDelta::seconds(60);

/// Cap used when the configured one does not fit a `TimeDelta`.
const DEFAULT_SETTLE_MAX: TimeDelta = TimeDelta::seconds(300);

pub struct Sweep {
    timing: Timing,
    mode: Mode,
    /// Whether owed invites are re-sent or dropped unsent.
    invites: Mode,
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
            settle_max: delta_or(
                config.session_reschedule_settle_max(),
                DEFAULT_SETTLE_MAX,
                "SESSION_RESCHEDULE_SETTLE_MAX_SECONDS",
            ),
            lease: coaching_session_pending_reschedule::LEASE,
        };

        let sendable = config.resend_api_key().is_some()
            && config.session_rescheduled_email_template_id().is_some();
        let mode = if sendable {
            Mode::Send
        } else {
            info!(
                "[coaching-session-pending-reschedule] reschedule emails are off; pending changes will be \
                 dropped unsent"
            );
            Mode::Drain
        };

        let invites = if config.resend_api_key().is_some()
            && config.session_scheduled_email_template_id().is_some()
            && config
                .recurring_sessions_scheduled_email_template_id()
                .is_some()
        {
            Mode::Send
        } else {
            info!(
                "[coaching-session-pending-reschedule] invite emails are off; owed invites will be \
                 dropped unsent"
            );
            Mode::Drain
        };

        Self {
            timing,
            mode,
            invites,
        }
    }

    /// Re-sends or drops one claimed owed invite; returns whether it was handled.
    async fn resend_invite(
        &self,
        ctx: &Context,
        claimed: &coaching_session_pending_invite::Claimed,
    ) -> bool {
        let db = &*ctx.db;
        let user_id = claimed.user_id;
        let resent = match (self.invites, claimed.target) {
            (Mode::Drain, _) => Ok(None),
            (Mode::Send, Target::Session(id)) => {
                crate::coaching_session::resend_invite_to(db, &ctx.config, id, user_id).await
            }
            (Mode::Send, Target::Series(id)) => {
                coaching_session_series::resend_invite_to(db, &ctx.config, id, user_id).await
            }
        };
        match resent {
            Ok(Some(Delivery::Failed)) => {
                warn!(
                    "[coaching-session-pending-reschedule] invite {:?} did not reach {user_id}",
                    claimed.target
                );
                retry_invite(db, claimed).await
            }
            Ok(_) => complete_invite(db, claimed).await,
            Err(e) => {
                warn!("[coaching-session-pending-reschedule] could not resend invite {:?} to {user_id}: {e:?}", claimed.target);
                retry_invite(db, claimed).await
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
            coaching_session_pending_invite::claim(&*ctx.db, now, Id::new_v4(), BATCH).await?;

        let mut handled = 0;
        for row in &claimed {
            if self.handle(ctx, row).await {
                handled += 1;
            }
        }
        for row in &owed {
            if self.resend_invite(ctx, row).await {
                handled += 1;
            }
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
    fn from_config_resends_invites_only_with_a_key_and_both_scheduled_templates() {
        let templates = [
            "--session-scheduled-email-template-id=scheduled_tpl",
            "--recurring-sessions-scheduled-email-template-id=recurring_tpl",
        ];
        let with = |args: &[&str]| Sweep::from_config(&config(args)).invites;

        assert_eq!(
            with(&[&["--resend-api-key=key"], &templates[..]].concat()),
            Mode::Send
        );
        assert_eq!(with(&templates), Mode::Drain);
        assert_eq!(with(&["--resend-api-key=key", templates[0]]), Mode::Drain);
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
