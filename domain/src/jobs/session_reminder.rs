//! Sends each coachee a reminder ahead of an upcoming coaching session.
//!
//! Each tick claims the sessions whose start has entered the lead window and emails the
//! coachee. "Due" is re-derived from the sessions table every tick, so a reschedule, a
//! cancellation, or a session booked five minutes ago all take effect on the next tick
//! with no work from the code paths that made the change:
//!
//! - **Rescheduled.** The claim holds the start it was sent for, so a moved session no
//!   longer matches its own claim and becomes due again. The coachee gets a fresh
//!   reminder for the new time.
//! - **Cancelled.** The row is gone, so nothing is due. No queued job survives to fire
//!   against a session that no longer exists.
//! - **Booked on short notice.** A session created less than the lead time before it
//!   starts is never reminded. The scheduled-session email already gave that heads-up,
//!   and a reminder minutes later would only repeat it. Rescheduling such a session into
//!   the future does not resurrect the reminder either, since the test is against when it
//!   was booked, not when it was last moved.
//!
//! See [`crate::jobs`] for why this is a sweep rather than an enqueued job.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use log::*;
use service::config::Config;

use crate::coaching_session;
use crate::emails;
use crate::error::Error;
use crate::jobs::{Context, FirstRun, Job, Outcome};

/// Most sessions one tick will claim.
///
/// Bounds the burst of Resend calls a single tick can make — a backlog (first run after
/// a long outage, say) drains over several ticks instead of hammering the API in one go.
/// Rows past the cap stay due and are picked up next tick.
const MAX_PER_TICK: u64 = 200;

pub struct Sweep {
    /// How far ahead of a session its reminder goes out.
    lead: chrono::Duration,
    interval: Duration,
}

impl Sweep {
    /// Build the sweep from config, or `None` when reminders are not configured.
    ///
    /// Returning `None` rather than a job that fails every tick is the point: without a
    /// template ID there is nothing to send, and a job that wakes only to log the same
    /// config error every few minutes buries real warnings.
    pub fn from_config(config: &Config) -> Option<Self> {
        config.session_reminder_email_template_id()?;

        let lead = match chrono::Duration::from_std(config.session_reminder_lead()) {
            Ok(lead) => lead,
            Err(e) => {
                warn!("[session-reminder] SESSION_REMINDER_LEAD_HOURS is out of range, reminders disabled: {e:?}");
                return None;
            }
        };

        Some(Self {
            lead,
            interval: config.session_reminder_poll_interval(),
        })
    }
}

#[async_trait]
impl Job for Sweep {
    fn name(&self) -> &'static str {
        "session-reminder"
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    /// Sessions that came due while the process was down are still worth reminding
    /// about, so the first sweep runs at startup rather than a poll interval later.
    fn first_run(&self) -> FirstRun {
        FirstRun::Immediately
    }

    async fn run(&self, ctx: &Context) -> Result<Outcome, Error> {
        let due = coaching_session::claim_due_reminders(
            &*ctx.db,
            Utc::now().naive_utc(),
            self.lead,
            MAX_PER_TICK,
        )
        .await?;

        if due.is_empty() {
            return Ok(Outcome::IDLE);
        }

        let mut sent = 0;
        // Skipping a recipient who lost access is a completed decision, not a failure, so
        // it counts as handled. Only a delivery that failed leaves the tick short.
        let mut skipped = 0;
        for reminder in &due {
            let session = &reminder.session;
            match emails::send_session_reminder(&ctx.db, &ctx.config, session).await {
                Ok(emails::ReminderOutcome::RecipientNoLongerAMember) => {
                    skipped += 1;
                    info!(
                        "[session-reminder] session {} not sent: user {} is no longer a \
                         member of that organization",
                        session.id, reminder.recipient_id
                    );
                    // Release rather than keep the claim, so re-adding them before the
                    // session restores the reminder instead of leaving it marked sent.
                    if let Err(e) = coaching_session::release_reminder_claim(
                        &*ctx.db,
                        session.id,
                        reminder.recipient_id,
                        reminder.claim_id,
                    )
                    .await
                    {
                        warn!(
                            "[session-reminder] could not release the claim on session {} \
                             for user {} after skipping them: {e:?}",
                            session.id, reminder.recipient_id
                        );
                    }
                }
                Ok(emails::ReminderOutcome::Sent) => {
                    sent += 1;
                    // The claim was stamped before this session was reloaded. When those
                    // disagree a reschedule landed in between, so record the start the
                    // email actually announced or the pair stays due and the same time
                    // goes out again next tick.
                    if session.date != reminder.claimed_for_start {
                        if let Err(e) = coaching_session::confirm_reminder_claim(
                            &*ctx.db,
                            session.id,
                            reminder.recipient_id,
                            reminder.claim_id,
                            session.date,
                        )
                        .await
                        {
                            warn!(
                                "[session-reminder] could not realign the claim on session \
                                 {} for user {}; it may be reminded again next tick: {e:?}",
                                session.id, reminder.recipient_id
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "[session-reminder] delivery failed for session {} to user {}: {e:?}",
                        session.id, reminder.recipient_id
                    );
                    // Hand the claim back so a later tick retries. Leaving it in place
                    // would turn one transient Resend failure into a silently dropped
                    // reminder.
                    if let Err(release_error) = coaching_session::release_reminder_claim(
                        &*ctx.db,
                        session.id,
                        reminder.recipient_id,
                        reminder.claim_id,
                    )
                    .await
                    {
                        warn!(
                            "[session-reminder] could not release the claim on session {} \
                             for user {} after a failed send; it will not be retried: \
                             {release_error:?}",
                            session.id, reminder.recipient_id
                        );
                    }
                }
            }
        }

        // `due.len()`, not the handled count: a tick that claimed reminders and failed to
        // deliver them has still found work, and reporting only successes reads as idle.
        Ok(Outcome::partial(sent + skipped, due.len() as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(args: &[&str]) -> Config {
        let mut argv = vec!["test"];
        argv.extend_from_slice(args);
        Config::from_args(argv)
    }

    /// No template means nothing can be sent, so the sweep must not be scheduled at all
    /// rather than waking every few minutes to log the same config error.
    #[test]
    fn from_config_is_none_without_a_template_id() {
        assert!(Sweep::from_config(&config(&[])).is_none());
    }

    #[test]
    fn from_config_reads_lead_and_interval() {
        let sweep = Sweep::from_config(&config(&[
            "--session-reminder-email-template-id=reminder_tpl",
            "--session-reminder-lead-hours=48",
            "--session-reminder-poll-minutes=5",
        ]))
        .expect("a configured template should produce a sweep");

        assert_eq!(sweep.lead, chrono::Duration::hours(48));
        assert_eq!(sweep.interval, Duration::from_secs(5 * 60));
    }

    #[test]
    fn from_config_defaults_to_a_24_hour_lead() {
        let sweep = Sweep::from_config(&config(&[
            "--session-reminder-email-template-id=reminder_tpl",
        ]))
        .expect("a configured template should produce a sweep");

        assert_eq!(sweep.lead, chrono::Duration::hours(24));
    }

    /// A zero poll interval would spin the sweep into a tight loop against the database,
    /// and a zero lead would make every session permanently out of window. Config clamps
    /// both; assert the clamp survives the trip through `from_config`.
    #[test]
    fn from_config_clamps_zeroes_to_something_survivable() {
        let sweep = Sweep::from_config(&config(&[
            "--session-reminder-email-template-id=reminder_tpl",
            "--session-reminder-lead-hours=0",
            "--session-reminder-poll-minutes=0",
        ]))
        .expect("a configured template should produce a sweep");

        assert_eq!(sweep.lead, chrono::Duration::hours(1));
        assert_eq!(sweep.interval, Duration::from_secs(60));
    }
}
