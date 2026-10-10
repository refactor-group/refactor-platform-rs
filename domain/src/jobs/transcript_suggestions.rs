//! Extracts suggested Actions from completed transcripts.
//!
//! A pending analysis row is written in the same transaction that completes a transcript.
//! Each tick claims a few of those rows (or ones whose claim went stale), asks the model,
//! and stores the verified suggestions. The claim is a conditional UPDATE, so replicas
//! divide the work, and completion is conditional on the claim still being the caller's,
//! so a worker whose claim was retaken stores nothing.
//!
//! See [`crate::jobs`] for why this is a sweep rather than an enqueued job.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use log::*;
use meeting_ai::traits::analysis;
use service::config::Config;

use crate::error::Error;
use crate::events::EventPublisher;
use crate::gateway::assembly_ai::llm_gateway;
use crate::jobs::{Context, FirstRun, Job, Outcome};
use crate::transcript_analysis::{self, claim, fail, find_claimable, release_for_retry};
use crate::transcript_suggestion::extract::{self, RunError};
use crate::transcript_suggestion::{Actions, Target};

/// Time between ticks.
const INTERVAL: Duration = Duration::from_secs(30);
/// A claim older than this is presumed abandoned and may be retaken.
const STALE_AFTER_MINUTES: i64 = 10;
/// Claims an analysis gets before it is failed for good.
const MAX_ATTEMPTS: i16 = 3;
/// Most analyses one tick takes; each is a model call.
const MAX_PER_TICK: u64 = 5;

/// The transcript suggestion sweep.
pub struct Sweep {
    provider: Arc<dyn analysis::Provider>,
    publisher: EventPublisher,
}

impl Sweep {
    /// Builds the sweep from config, or `None` when no AssemblyAI key is configured.
    pub fn from_config(config: &Config, publisher: EventPublisher) -> Option<Self> {
        let api_key = config.assembly_ai_api_key()?;
        let provider = llm_gateway::Provider::new(
            &api_key,
            config.assembly_ai_llm_base_url(),
            config.assembly_ai_llm_model(),
        )
        .inspect_err(|e| {
            warn!(
                "[transcript-suggestions] building the LLM Gateway client failed, disabled: {e:?}"
            )
        })
        .ok()?;

        Some(Self {
            provider: Arc::new(provider),
            publisher,
        })
    }

    /// Runs one claimed analysis and records how it ended; true when it was handled.
    async fn process(&self, ctx: &Context, analysis: &transcript_analysis::Model) -> bool {
        let result = match analysis.target {
            Target::Action => {
                extract::run::<Actions>(&ctx.db, self.provider.as_ref(), &self.publisher, analysis)
                    .await
            }
        };

        let (retryable, message) = match result {
            Ok(extract::Outcome::Stored(_)) => return true,
            Ok(extract::Outcome::Superseded) => {
                info!(
                    "[transcript-suggestions] analysis {} was superseded by another worker",
                    analysis.id
                );
                return true;
            }
            Err(RunError::Provider(e)) => (
                !matches!(
                    e,
                    meeting_ai::Error::Authentication(_) | meeting_ai::Error::Configuration(_)
                ),
                format!("provider error: {}", provider_error_kind(&e)),
            ),
            Err(RunError::Verification(_)) => (true, "model output failed verification".into()),
            Err(RunError::Domain(e)) => (false, format!("domain error: {:?}", e.error_kind)),
        };

        let give_up = !retryable || analysis.attempts >= MAX_ATTEMPTS;
        warn!(
            "[transcript-suggestions] analysis {} attempt {} failed ({message}); {}",
            analysis.id,
            analysis.attempts,
            if give_up { "failing it" } else { "will retry" }
        );
        let recorded = if give_up {
            fail(ctx.db.as_ref(), analysis.id, message).await
        } else {
            release_for_retry(ctx.db.as_ref(), analysis.id, message).await
        };
        if let Err(e) = recorded {
            warn!(
                "[transcript-suggestions] recording the failure of analysis {} failed: {e:?}",
                analysis.id
            );
        }
        false
    }
}

/// The error's variant only; its message may echo provider or model content.
fn provider_error_kind(err: &meeting_ai::Error) -> &'static str {
    match err {
        meeting_ai::Error::Authentication(_) => "Authentication",
        meeting_ai::Error::Network(_) => "Network",
        meeting_ai::Error::Configuration(_) => "Configuration",
        meeting_ai::Error::Provider(_) => "Provider",
        meeting_ai::Error::Timeout(_) => "Timeout",
        meeting_ai::Error::NotFound(_) => "NotFound",
        meeting_ai::Error::RateLimited { .. } => "RateLimited",
        meeting_ai::Error::Serialization(_) => "Serialization",
        meeting_ai::Error::Deserialization(_) => "Deserialization",
        meeting_ai::Error::Other(_) => "Other",
    }
}

#[async_trait]
impl Job for Sweep {
    fn name(&self) -> &'static str {
        "transcript-suggestions"
    }

    fn interval(&self) -> Duration {
        INTERVAL
    }

    fn first_run(&self) -> FirstRun {
        FirstRun::Immediately
    }

    async fn run(&self, ctx: &Context) -> Result<Outcome, Error> {
        let now = Utc::now().fixed_offset();
        let stale_before = now - chrono::Duration::minutes(STALE_AFTER_MINUTES);
        let due = find_claimable(ctx.db.as_ref(), stale_before, MAX_ATTEMPTS, MAX_PER_TICK).await?;

        let (mut processed, mut attempted) = (0, 0);
        for candidate in due {
            let claimed = match claim(
                ctx.db.as_ref(),
                candidate.id,
                now,
                stale_before,
                MAX_ATTEMPTS,
            )
            .await
            {
                Ok(Some(claimed)) => claimed,
                Ok(None) => continue,
                Err(e) => {
                    warn!(
                        "[transcript-suggestions] claiming analysis {} failed: {e:?}",
                        candidate.id
                    );
                    attempted += 1;
                    continue;
                }
            };
            attempted += 1;
            if self.process(ctx, &claimed).await {
                processed += 1;
            }
        }

        Ok(Outcome::partial(processed, attempted))
    }
}

#[cfg(test)]
#[path = "transcript_suggestions_sqlite_tests.rs"]
mod sqlite_tests;
