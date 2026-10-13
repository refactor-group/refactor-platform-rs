//! Pass rates per variant and case, compared against the checked-in baseline.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::case::{Set, FIXTURES};
use super::score::Finding;

/// The checked-in pass rates of the last accepted run.
pub(super) const BASELINE: &str = "baseline.json";

/// One model call and how it scored.
#[derive(Debug)]
pub(super) struct Run {
    /// Findings, or why the call or verification failed.
    pub result: Result<Vec<Finding>, String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl Run {
    pub(super) fn passed(&self) -> bool {
        self.result
            .as_ref()
            .is_ok_and(|findings| findings.iter().all(|f| f.errors() == 0))
    }
}

/// Every run of one variant.
#[derive(Debug)]
pub(super) struct Outcome {
    pub set: Set,
    pub case_id: String,
    pub variant_id: String,
    pub runs: Vec<Run>,
}

impl Outcome {
    pub(super) fn rate(&self) -> Rate {
        Rate {
            passed: self.runs.iter().filter(|r| r.passed()).count(),
            runs: self.runs.len(),
        }
    }
}

/// Runs passed out of runs made; `"2/3"` in the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(super) struct Rate {
    pub passed: usize,
    pub runs: usize,
}

impl Rate {
    /// True when this rate is lower than `baseline`'s.
    pub(super) fn below(self, baseline: Rate) -> bool {
        self.passed * baseline.runs < baseline.passed * self.runs
    }

    /// True when this rate fell more than one run's worth below `baseline`'s; a one-run dip is
    /// within the model's run-to-run variance.
    pub(super) fn regressed_from(self, baseline: Rate) -> bool {
        baseline.passed * self.runs > self.passed * baseline.runs + baseline.runs
    }

    fn show(self) -> String {
        format!("{}/{}", self.passed, self.runs)
    }
}

impl TryFrom<String> for Rate {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value
            .split_once('/')
            .and_then(|(passed, runs)| {
                Some(Rate {
                    passed: passed.trim().parse().ok()?,
                    runs: runs.trim().parse().ok()?,
                })
            })
            .filter(|rate| rate.passed <= rate.runs)
            .ok_or_else(|| format!("{value:?} is not a pass rate like \"2/3\""))
    }
}

impl From<Rate> for String {
    fn from(rate: Rate) -> Self {
        rate.show()
    }
}

/// Last accepted pass rate per variant id.
pub(super) type Baseline = BTreeMap<String, Rate>;

pub(super) fn read_baseline() -> Result<Baseline, String> {
    let path = Path::new(FIXTURES).join(BASELINE);
    if !path.exists() {
        return Ok(Baseline::new());
    }
    fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Replaces the rates of the variants that ran and keeps the rest.
pub(super) fn write_baseline(mut baseline: Baseline, outcomes: &[Outcome]) -> Result<(), String> {
    baseline.extend(outcomes.iter().map(|o| (o.variant_id.clone(), o.rate())));
    let json = serde_json::to_string_pretty(&baseline).map_err(|e| e.to_string())?;
    fs::write(Path::new(FIXTURES).join(BASELINE), json + "\n").map_err(|e| e.to_string())
}

/// Variant ids whose pass rate regressed against the baseline.
pub(super) fn regressions(outcomes: &[Outcome], baseline: &Baseline) -> Vec<String> {
    outcomes
        .iter()
        .filter(|o| {
            baseline
                .get(&o.variant_id)
                .is_some_and(|b| o.rate().regressed_from(*b))
        })
        .map(|o| o.variant_id.clone())
        .collect()
}

/// A case passes a run only when every variant passes that run.
pub(super) fn case_rate(variants: &[&Outcome]) -> Rate {
    let runs = variants.iter().map(|o| o.runs.len()).min().unwrap_or(0);
    Rate {
        passed: (0..runs)
            .filter(|&i| variants.iter().all(|o| o.runs[i].passed()))
            .count(),
        runs,
    }
}

/// What the run cost, from per-million-token prices.
pub(super) struct Pricing {
    pub model: String,
    pub input_usd_per_mtok: f64,
    pub output_usd_per_mtok: f64,
}

/// The full report as Markdown.
pub(super) fn render(outcomes: &[Outcome], baseline: &Baseline, pricing: &Pricing) -> String {
    let runs = outcomes.iter().map(|o| o.runs.len()).max().unwrap_or(0);
    let mut out = format!(
        "# Suggested-action eval\n\nModel `{}`, {runs} run(s) per variant.\n",
        pricing.model
    );

    for set in Set::ALL {
        let in_set: Vec<&Outcome> = outcomes.iter().filter(|o| o.set == set).collect();
        if in_set.is_empty() {
            continue;
        }
        let mut cases: BTreeMap<&str, Vec<&Outcome>> = BTreeMap::new();
        for outcome in &in_set {
            cases.entry(&outcome.case_id).or_default().push(outcome);
        }
        let passing = cases
            .values()
            .filter(|v| {
                let rate = case_rate(v);
                rate.passed == rate.runs
            })
            .count();
        let _ = writeln!(
            out,
            "\n## {} ({passing}/{} cases pass every run)\n\n```",
            set.dir(),
            cases.len()
        );
        for (case_id, variants) in &cases {
            let _ = writeln!(out, "{case_id:<34} {}", case_rate(variants).show());
            for outcome in variants {
                write_variant(&mut out, outcome, baseline);
            }
        }
        out.push_str("```\n");
    }

    let regressed = regressions(outcomes, baseline);
    let _ = writeln!(
        out,
        "\n## Against the baseline\n\n{}",
        if regressed.is_empty() {
            "No variant regressed (a one-run dip is shown above but does not fail).".to_string()
        } else {
            format!("**Regressed:** {}", regressed.join(", "))
        }
    );

    let calls = outcomes.iter().map(|o| o.runs.len()).sum::<usize>();
    let input: u64 = all_runs(outcomes).map(|r| u64::from(r.input_tokens)).sum();
    let output: u64 = all_runs(outcomes).map(|r| u64::from(r.output_tokens)).sum();
    let cost = (input as f64 * pricing.input_usd_per_mtok
        + output as f64 * pricing.output_usd_per_mtok)
        / 1_000_000.0;
    let _ = writeln!(
        out,
        "\n## Usage\n\n{calls} calls, {input} input and {output} output tokens, about ${cost:.2} \
         (${:.2} per pass over every variant).",
        if runs == 0 { 0.0 } else { cost / runs as f64 }
    );
    out
}

fn write_variant(out: &mut String, outcome: &Outcome, baseline: &Baseline) {
    let rate = outcome.rate();
    let name = outcome
        .variant_id
        .rsplit_once('.')
        .map_or(outcome.variant_id.as_str(), |(_, name)| name);
    let against = match baseline.get(&outcome.variant_id) {
        Some(b) if rate.regressed_from(*b) => format!("  REGRESSED from {}", b.show()),
        Some(b) if rate.below(*b) => format!("  dipped from {}", b.show()),
        Some(b) if b.below(rate) => format!("  improved from {}", b.show()),
        Some(_) => String::new(),
        None => "  new".to_string(),
    };
    let _ = writeln!(out, "  {name:<32} {}{against}", rate.show());
    for (i, run) in outcome.runs.iter().enumerate().filter(|(_, r)| !r.passed()) {
        let _ = writeln!(out, "    run {}:", i + 1);
        match &run.result {
            Err(e) => {
                let _ = writeln!(out, "      ERROR {e}");
            }
            Ok(findings) => findings.iter().filter(|f| f.errors() > 0).for_each(|f| {
                let _ = writeln!(out, "      {f}");
            }),
        }
    }
}

fn all_runs(outcomes: &[Outcome]) -> impl Iterator<Item = &Run> {
    outcomes.iter().flat_map(|o| o.runs.iter())
}
