//! Recurring background work.
//!
//! Every job in this module is a *periodic sweep*: it wakes on a fixed interval, asks
//! the database what is due right now, and acts on it. There is no separate queue and
//! nothing is enqueued ahead of time — Postgres is the queue, and each job's `run`
//! contains the predicate that defines "due".
//!
//! That shape is deliberate. A durable job queue would have to be told, at enqueue time,
//! what work exists; every later edit — a session moved, a session cancelled, a
//! relationship ended — would then need a matching dequeue or re-enqueue, and any missed
//! one leaves a stale job that fires against reality. A sweep re-derives the work list
//! from current rows on every tick, so it is correct by construction after any edit and
//! needs no compensating logic. The cost is latency bounded by the tick interval, which
//! for work measured in hours is free.
//!
//! Durability and multi-replica safety come from the queries, not from a broker: a job
//! that must not double-act claims its rows in the same statement that selects them, so
//! two backend replicas running the same sweep divide the work instead of duplicating
//! it. `entity_api::coaching_session::claim_due_reminders` is the canonical example: it
//! upserts into a claim table and lets the unique index arbitrate, returning only the
//! rows that caller won. See `docs/architecture/background_jobs.md` for why that beats
//! locking the source row.
//!
//! # Adding a job
//!
//! Implement [`Job`] on a unit-ish struct in its own submodule, then register it in
//! `web::init_server` via [`Scheduler::spawn`]. A job that has nothing to do should
//! return [`Outcome::IDLE`] rather than logging, so quiet ticks stay quiet. A job that
//! could not finish what it found returns [`Outcome::partial`], which the scheduler
//! reports at WARN so a failing tick is never mistaken for an idle one.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::error::Error;

pub mod password_reset;
pub mod session_reminder;

/// What every job is handed on each tick.
///
/// Cloned once per job at spawn time, so a job owns its handle to the pool and config
/// for the lifetime of the process rather than borrowing across `await` points.
#[derive(Clone)]
pub struct Context {
    pub db: Arc<DatabaseConnection>,
    pub config: Config,
}

/// What one tick accomplished, for logging.
///
/// `processed` and `attempted` are job-defined units: reminders sent, rows deleted. They
/// are separate because a tick where everything failed still found work, and reporting it
/// as idle hides the job from anyone reading the log at INFO.
pub struct Outcome {
    pub processed: u64,
    pub attempted: u64,
}

impl Outcome {
    /// Nothing was due this tick.
    pub const IDLE: Self = Self {
        processed: 0,
        attempted: 0,
    };

    /// Every item found was handled.
    pub fn processed(count: u64) -> Self {
        Self {
            processed: count,
            attempted: count,
        }
    }

    /// Some of what was found could not be handled. The job has already logged why.
    pub fn partial(processed: u64, attempted: u64) -> Self {
        Self {
            processed,
            attempted,
        }
    }
}

/// Whether a job's first tick fires at startup or one interval later.
#[derive(Clone, Copy)]
pub enum FirstRun {
    /// Run as soon as the server is up. Right for work that may have accumulated while
    /// the process was down.
    Immediately,
    /// Wait out one full interval first. Right for maintenance that gains nothing from
    /// running during startup, when the process is already busy.
    AfterInterval,
}

/// One unit of recurring background work.
#[async_trait]
pub trait Job: Send + Sync + 'static {
    /// Short, stable identifier used as the log prefix. Kebab-case by convention.
    fn name(&self) -> &'static str;

    /// How long the scheduler waits between ticks.
    fn interval(&self) -> Duration;

    fn first_run(&self) -> FirstRun {
        FirstRun::AfterInterval
    }

    /// Do one tick's worth of work.
    ///
    /// Returning `Err` is expected and survivable: the scheduler logs it and ticks
    /// again. Implementations must therefore be safe to re-enter after a partial
    /// failure — do not leave work in a state only a successful tick can undo.
    async fn run(&self, ctx: &Context) -> Result<Outcome, Error>;
}

/// Owns the tokio tasks running the registered jobs.
///
/// One task per job, so a job that blocks on a slow query or a hung HTTP call delays
/// only itself. Tasks run until the process exits; there is no cancellation channel
/// because there is no shutdown path that needs one — dropping the handles detaches
/// the tasks and the runtime tears them down with the process.
pub struct Scheduler {
    context: Context,
    handles: Vec<JoinHandle<()>>,
}

impl Scheduler {
    pub fn new(db: Arc<DatabaseConnection>, config: Config) -> Self {
        Self {
            context: Context { db, config },
            handles: Vec::new(),
        }
    }

    /// Register a job and start ticking it.
    pub fn spawn(&mut self, job: impl Job) -> &mut Self {
        let context = self.context.clone();
        let job = Arc::new(job);
        let name = job.name();
        let interval = job.interval();

        info!("[{name}] scheduled every {}s", interval.as_secs());

        self.handles.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // A tick that comes due while `run` is still working must not queue up a
            // burst of catch-up ticks the moment it returns; the sweep would just find
            // the same empty result set several times in a row.
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

            // `tokio::time::interval` always yields its first tick immediately, so a
            // job that wants to wait out a period has to consume that one up front.
            if matches!(job.first_run(), FirstRun::AfterInterval) {
                ticker.tick().await;
            }

            loop {
                ticker.tick().await;
                match job.run(&context).await {
                    Ok(outcome) if outcome.processed == outcome.attempted => {
                        if outcome.processed > 0 {
                            info!("[{name}] processed {} item(s)", outcome.processed);
                        } else {
                            debug!("[{name}] nothing due");
                        }
                    }
                    // Reported at WARN even when some succeeded: a tick that found work
                    // and could not finish it must not read as an idle one.
                    Ok(outcome) => warn!(
                        "[{name}] processed {} of {} item(s)",
                        outcome.processed, outcome.attempted
                    ),
                    Err(e) => warn!("[{name}] tick failed: {e:?}"),
                }
            }
        }));

        self
    }

    /// Hand the task handles to the caller, which is expected to keep them alive for
    /// as long as the server runs.
    pub fn into_handles(self) -> Vec<JoinHandle<()>> {
        self.handles
    }
}

#[cfg(all(test, feature = "mock"))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use sea_orm::{DatabaseBackend, MockDatabase};
    use tokio::sync::mpsc::{self, UnboundedSender};
    use tokio::time::Instant;

    use super::*;
    use crate::error::{DomainErrorKind, InternalErrorKind};

    /// The clock is paused in these tests, so `rx.recv().await` parks the test task, the
    /// runtime goes idle, and tokio jumps the clock to the job's next tick. Every tick
    /// therefore lands at an exact instant, and nothing waits in real time.
    const TICK: Duration = Duration::from_secs(600);

    /// A tick that found work and failed all of it reported the same count as one that
    /// found nothing, so the scheduler logged "nothing due" while warning about every
    /// failure. `attempted` is what separates them.
    #[test]
    fn a_wholly_failed_tick_is_not_idle() {
        let failed = Outcome::partial(0, 3);

        assert_ne!(
            failed.attempted, failed.processed,
            "a tick that handled none of what it found must be distinguishable from idle"
        );
        assert_eq!(Outcome::IDLE.attempted, 0, "idle found nothing to attempt");
    }

    /// A job that handled everything it found must not warn. `processed` alone cannot say
    /// so: 0 of 0 and 0 of 3 both have `processed == 0`.
    #[test]
    fn a_fully_handled_tick_matches_what_it_attempted() {
        assert_eq!(
            Outcome::processed(5).attempted,
            5,
            "handling everything found means attempted equals processed, which is what \
             keeps a healthy tick out of the warning path"
        );
        assert_eq!(Outcome::IDLE.processed, Outcome::IDLE.attempted);
    }

    /// Reports the instant of every tick, so a test can assert *when* it ran rather than
    /// only that it ran.
    struct Recorder {
        ticks: UnboundedSender<Instant>,
        runs: Arc<AtomicU64>,
        first_run: FirstRun,
        /// Ticks fail until this many have run, to prove the loop survives errors.
        fail_first: u64,
    }

    #[async_trait]
    impl Job for Recorder {
        fn name(&self) -> &'static str {
            "recorder"
        }

        fn interval(&self) -> Duration {
            TICK
        }

        fn first_run(&self) -> FirstRun {
            self.first_run
        }

        async fn run(&self, _ctx: &Context) -> Result<Outcome, Error> {
            let run = self.runs.fetch_add(1, Ordering::SeqCst);
            let _ = self.ticks.send(Instant::now());

            if run < self.fail_first {
                return Err(Error {
                    source: None,
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                        "deliberate test failure".to_string(),
                    )),
                });
            }

            Ok(Outcome::processed(1))
        }
    }

    fn scheduler() -> Scheduler {
        Scheduler::new(
            Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection()),
            Config::from_args(["test"]),
        )
    }

    fn recorder(
        first_run: FirstRun,
        fail_first: u64,
    ) -> (Recorder, mpsc::UnboundedReceiver<Instant>) {
        let (ticks, rx) = mpsc::unbounded_channel();
        (
            Recorder {
                ticks,
                runs: Arc::new(AtomicU64::new(0)),
                first_run,
                fail_first,
            },
            rx,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn first_run_immediately_ticks_at_startup() {
        let start = Instant::now();
        let (job, mut ticks) = recorder(FirstRun::Immediately, 0);

        let _handles = {
            let mut scheduler = scheduler();
            scheduler.spawn(job);
            scheduler.into_handles()
        };

        let first = ticks.recv().await.expect("job never ticked");
        assert_eq!(first - start, Duration::ZERO);
    }

    /// `tokio::time::interval` yields its first tick immediately, so `AfterInterval` is
    /// only honoured if the scheduler consumes that freebie. Without that, maintenance
    /// jobs would all fire during startup.
    #[tokio::test(start_paused = true)]
    async fn first_run_after_interval_waits_out_one_period() {
        let start = Instant::now();
        let (job, mut ticks) = recorder(FirstRun::AfterInterval, 0);

        let _handles = {
            let mut scheduler = scheduler();
            scheduler.spawn(job);
            scheduler.into_handles()
        };

        let first = ticks.recv().await.expect("job never ticked");
        assert_eq!(first - start, TICK);
    }

    #[tokio::test(start_paused = true)]
    async fn ticks_repeat_on_the_declared_interval() {
        let (job, mut ticks) = recorder(FirstRun::Immediately, 0);

        let _handles = {
            let mut scheduler = scheduler();
            scheduler.spawn(job);
            scheduler.into_handles()
        };

        let first = ticks.recv().await.expect("job never ticked");
        let second = ticks.recv().await.expect("job ticked only once");
        let third = ticks.recv().await.expect("job ticked only twice");

        assert_eq!(second - first, TICK);
        assert_eq!(third - second, TICK);
    }

    /// A failing tick is survivable by contract — the scheduler logs it and ticks again.
    /// A job whose first two runs error must still reach a third.
    #[tokio::test(start_paused = true)]
    async fn a_failing_tick_does_not_kill_the_job() {
        let (job, mut ticks) = recorder(FirstRun::Immediately, 2);

        let _handles = {
            let mut scheduler = scheduler();
            scheduler.spawn(job);
            scheduler.into_handles()
        };

        // Runs 0 and 1 return `Err`; reaching run 2 proves the loop outlived both.
        for tick in 0..3 {
            ticks
                .recv()
                .await
                .unwrap_or_else(|| panic!("job stopped ticking before tick {tick}"));
        }
    }

    /// Each job gets its own task, so two registered jobs both run.
    #[tokio::test(start_paused = true)]
    async fn every_registered_job_runs() {
        let (first_job, mut first_ticks) = recorder(FirstRun::Immediately, 0);
        let (second_job, mut second_ticks) = recorder(FirstRun::Immediately, 0);

        let _handles = {
            let mut scheduler = scheduler();
            scheduler.spawn(first_job);
            scheduler.spawn(second_job);
            scheduler.into_handles()
        };

        first_ticks.recv().await.expect("first job never ticked");
        second_ticks.recv().await.expect("second job never ticked");
    }
}
