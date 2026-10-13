//! The opt-in run against the real model.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use meeting_ai::traits::analysis::Provider as _;
use meeting_ai::types::analysis::{Completion, Request};
use service::config::Config;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::case::{self, Variant};
use super::report::{self, Outcome, Pricing, Run};
use super::score::{self, Got};
use crate::gateway::assembly_ai::llm_gateway;
use crate::transcript_suggestion::actions::Role;
use crate::transcript_suggestion::extract::request;
use crate::transcript_suggestion::{verify_output, Actions, Refs};
use crate::Id;

const DEFAULT_RUNS: usize = 3;
const DEFAULT_CONCURRENCY: usize = 4;
/// Rate-limited calls are retried after the wait the gateway asks for, at most this many times.
const RATE_LIMIT_RETRIES: u32 = 8;
/// Claude Sonnet 4.6 list prices, US dollars per million tokens.
const DEFAULT_INPUT_USD_PER_MTOK: f64 = 3.0;
const DEFAULT_OUTPUT_USD_PER_MTOK: f64 = 15.0;

/// Runs every variant `SUGGESTION_EVAL_RUNS` times and fails when one regressed against the baseline.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls the real model; see docs/test-plans/transcript_suggestions_eval.md"]
async fn suggestion_eval() {
    service::load_env_file();
    let config = Config::default();
    let api_key = config
        .assembly_ai_api_key()
        .expect("set ASSEMBLY_AI_API_KEY in the environment or .env");
    let runs = setting("SUGGESTION_EVAL_RUNS", DEFAULT_RUNS);
    let concurrency = setting("SUGGESTION_EVAL_CONCURRENCY", DEFAULT_CONCURRENCY);
    let filters: Vec<String> = env::var("SUGGESTION_EVAL_CASES")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect();
    let pricing = Pricing {
        model: config.assembly_ai_llm_model().to_string(),
        input_usd_per_mtok: setting(
            "SUGGESTION_EVAL_INPUT_USD_PER_MTOK",
            DEFAULT_INPUT_USD_PER_MTOK,
        ),
        output_usd_per_mtok: setting(
            "SUGGESTION_EVAL_OUTPUT_USD_PER_MTOK",
            DEFAULT_OUTPUT_USD_PER_MTOK,
        ),
    };

    let variants: Arc<Vec<Variant>> = Arc::new(
        case::load_all()
            .expect("fixtures load")
            .into_iter()
            .filter(|v| filters.is_empty() || filters.iter().any(|f| selects(f, v)))
            .collect(),
    );
    assert!(
        !variants.is_empty(),
        "SUGGESTION_EVAL_CASES matched no case"
    );
    let provider = llm_gateway::Provider::new(
        &api_key,
        config.assembly_ai_llm_base_url(),
        config.assembly_ai_llm_model(),
    )
    .expect("LLM Gateway client builds");
    let out_dir = output_dir();

    let permits = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut tasks = JoinSet::new();
    for index in 0..variants.len() {
        for run in 0..runs {
            let (variants, provider, permits) =
                (variants.clone(), provider.clone(), permits.clone());
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await.expect("semaphore open");
                let (scored, raw) = run_once(&provider, &variants[index]).await;
                (index, run, scored, raw)
            });
        }
    }
    let mut results: Vec<(usize, usize, Run, Option<serde_json::Value>)> = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        results.push(joined.expect("eval task finished"));
    }
    results.sort_by_key(|(index, run, _, _)| (*index, *run));

    for (index, run, _, raw) in &results {
        if let Some(raw) = raw {
            let path = out_dir.join(format!("{}.run{}.json", variants[*index].id(), run + 1));
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&path, serde_json::to_string_pretty(raw).unwrap_or_default());
        }
    }

    let mut outcomes: Vec<Outcome> = variants
        .iter()
        .map(|v| Outcome {
            set: v.set,
            case_id: v.case_id(),
            variant_id: v.id(),
            runs: Vec::new(),
        })
        .collect();
    for (index, _, scored, _) in results {
        outcomes[index].runs.push(scored);
    }

    let baseline = report::read_baseline().expect("baseline reads");
    let text = report::render(&outcomes, &baseline, &pricing);
    println!("{text}");
    let report_path = out_dir.join("report.md");
    if fs::create_dir_all(&out_dir)
        .and_then(|_| fs::write(&report_path, &text))
        .is_ok()
    {
        println!("Report and raw answers written under {}", out_dir.display());
    }

    if env::var("SUGGESTION_EVAL_UPDATE_BASELINE").is_ok_and(|v| v == "1") {
        report::write_baseline(baseline, &outcomes).expect("baseline writes");
        println!("Baseline updated; review and commit it with the change it measures.");
        return;
    }
    let regressed = report::regressions(&outcomes, &baseline);
    assert!(
        regressed.is_empty(),
        "pass rate fell below the baseline for: {}",
        regressed.join(", ")
    );
}

/// One model call for one variant, scored, with the model's raw answer.
async fn run_once(
    provider: &llm_gateway::Provider,
    variant: &Variant,
) -> (Run, Option<serde_json::Value>) {
    let transcript = &variant.transcript;
    let input = transcript.input();
    let session = transcript.session(&input);
    let refs = Refs::new(&session);

    let completion = match complete(provider, request::<Actions>(&session)).await {
        Ok(completion) => completion,
        Err(e) => {
            let run = Run {
                result: Err(format!("model call failed: {e:?}")),
                input_tokens: 0,
                output_tokens: 0,
            };
            return (run, None);
        }
    };
    let result = verify_output::<Actions>(&completion.output, &session, &refs)
        .map_err(|e| format!("verification failed: {e:?}"))
        .map(|verified| {
            let got: Vec<Got> = verified
                .into_iter()
                .map(|v| Got {
                    segment: transcript
                        .segment_label(&input, v.segment_id)
                        .unwrap_or("?")
                        .to_string(),
                    assignees: v
                        .proposal
                        .action
                        .assignee_ids
                        .iter()
                        .map(|id| role_of(*id, &input))
                        .collect(),
                    due: v
                        .proposal
                        .action
                        .action
                        .due_by
                        .map(|due| due.with_timezone(&transcript.timezone).date_naive()),
                    due_phrase: v.proposal.due_phrase,
                    duplicate_of: v
                        .duplicate_of_id
                        .and_then(|id| transcript.existing_label(&input, id))
                        .map(str::to_string),
                    quote: v.quote,
                    body: v.proposal.action.action.body.unwrap_or_default(),
                })
                .collect();
            score::score(&variant.expected, &got, transcript)
        });
    let run = Run {
        result,
        input_tokens: completion.usage.input_tokens,
        output_tokens: completion.usage.output_tokens,
    };
    (run, Some(completion.output))
}

/// One completion, waiting out the gateway's rate limit up to `RATE_LIMIT_RETRIES` times.
async fn complete(
    provider: &llm_gateway::Provider,
    request: Request,
) -> Result<Completion, meeting_ai::Error> {
    let mut retries = 0;
    loop {
        match provider.complete(request.clone()).await {
            Err(meeting_ai::Error::RateLimited {
                retry_after_seconds,
            }) if retries < RATE_LIMIT_RETRIES => {
                retries += 1;
                tokio::time::sleep(Duration::from_secs(retry_after_seconds.max(1))).await;
            }
            result => return result,
        }
    }
}

fn role_of(id: Id, input: &case::Input) -> Role {
    if id == input.people.coach_id {
        Role::Coach
    } else {
        Role::Coachee
    }
}

/// A filter selects a variant by a prefix of its id (`clean/f05`) or its case (`f05`).
fn selects(filter: &str, variant: &Variant) -> bool {
    variant.id().starts_with(filter) || variant.case.starts_with(filter)
}

fn setting<T: std::str::FromStr>(name: &str, default: T) -> T {
    env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// `target/suggestion_eval`, or under `CARGO_TARGET_DIR` when set.
fn output_dir() -> PathBuf {
    env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap_or(Path::new("."))
                .join("target")
        })
        .join("suggestion_eval")
}
