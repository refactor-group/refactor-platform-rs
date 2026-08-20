//! Daily sweep of the `password_reset_attempts` audit table.
//!
//! Retention has to outlive the 24-hour daily-cap rate-limit window, which reads recent
//! rows; the rest of the retention period is kept for security forensics. See
//! `docs/architecture/password_reset.md` and [`crate::password_reset::sweep_old_attempts`]
//! for the policy and for why the sweep runs in-process rather than as an external cron.

use std::time::Duration;

use async_trait::async_trait;

use crate::error::Error;
use crate::jobs::{Context, Job, Outcome};
use crate::password_reset;

const SWEEP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

const RETENTION_DAYS: i64 = 30;

pub struct Sweep {
    retention_days: i64,
}

impl Default for Sweep {
    fn default() -> Self {
        Self {
            retention_days: RETENTION_DAYS,
        }
    }
}

impl Sweep {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Job for Sweep {
    fn name(&self) -> &'static str {
        "password-reset-sweep"
    }

    fn interval(&self) -> Duration {
        SWEEP_INTERVAL
    }

    async fn run(&self, ctx: &Context) -> Result<Outcome, Error> {
        password_reset::sweep_old_attempts(&ctx.db, self.retention_days)
            .await
            .map(Outcome::processed)
    }
}
