//! Pass rates, case rates and regressions against the baseline.

use super::case::Set;
use super::report::{case_rate, regressions, render, Baseline, Outcome, Pricing, Rate, Run};
use super::score::Finding;

fn run(passed: bool) -> Run {
    let finding = if passed {
        Finding::Ok("s1".into())
    } else {
        Finding::Miss("s1".into())
    };
    Run {
        result: Ok(vec![finding]),
        input_tokens: 1_000_000,
        output_tokens: 100_000,
    }
}

fn outcome(variant: &str, passes: &[bool]) -> Outcome {
    Outcome {
        set: Set::Clean,
        case_id: "clean/f05".into(),
        variant_id: format!("clean/f05.{variant}"),
        runs: passes.iter().map(|p| run(*p)).collect(),
    }
}

fn rate(passed: usize, runs: usize) -> Rate {
    Rate { passed, runs }
}

#[test]
fn an_answer_that_fails_verification_never_passes() {
    let failed = Run {
        result: Err("verification failed".into()),
        input_tokens: 0,
        output_tokens: 0,
    };
    assert!(!failed.passed());
    assert!(run(true).passed());
}

#[test]
fn compares_rates_with_different_run_counts() {
    assert!(rate(2, 3).below(rate(3, 3)));
    assert!(rate(4, 6).below(rate(3, 3)));
    assert!(!rate(4, 6).below(rate(2, 3)));
    assert!(!rate(5, 5).below(rate(3, 3)));
}

#[test]
fn only_a_drop_of_more_than_one_run_regresses() {
    assert!(!rate(2, 3).regressed_from(rate(3, 3)));
    assert!(rate(1, 3).regressed_from(rate(3, 3)));
    assert!(!rate(0, 3).regressed_from(rate(1, 3)));
    assert!(rate(3, 5).regressed_from(rate(3, 3)));
    assert!(!rate(4, 5).regressed_from(rate(3, 3)));
}

#[test]
fn baseline_rates_read_and_write_as_fractions() {
    let baseline: Baseline = serde_json::from_str(r#"{"clean/f05.v1": "2/3"}"#).expect("parses");
    assert_eq!(baseline["clean/f05.v1"], rate(2, 3));
    assert_eq!(
        serde_json::to_string(&baseline).expect("writes"),
        r#"{"clean/f05.v1":"2/3"}"#
    );
    assert!(serde_json::from_str::<Baseline>(r#"{"x": "4/3"}"#).is_err());
    assert!(serde_json::from_str::<Baseline>(r#"{"x": "0/0"}"#).is_err());
}

#[test]
fn a_case_passes_a_run_only_when_every_variant_does() {
    let v1 = outcome("v1", &[true, true, false]);
    let v2 = outcome("v2", &[true, false, true]);

    assert_eq!(case_rate(&[&v1, &v2]), rate(1, 3));
}

#[test]
fn regressions_are_drops_against_known_variants() {
    let outcomes = [
        outcome("v1", &[true, false, false]),
        outcome("v2", &[true, true, true]),
        outcome("v3", &[true, false, true]),
    ];
    let baseline: Baseline = [
        ("clean/f05.v1".to_string(), rate(3, 3)),
        ("clean/f05.v2".to_string(), rate(2, 3)),
        ("clean/f05.v3".to_string(), rate(3, 3)),
    ]
    .into();

    assert_eq!(regressions(&outcomes, &baseline), ["clean/f05.v1"]);
    assert!(regressions(&outcomes, &Baseline::new()).is_empty());
}

#[test]
fn the_report_shows_rates_findings_regressions_and_cost() {
    let outcomes = [
        outcome("v1", &[false, false, true]),
        outcome("v2", &[true, true, true]),
        outcome("v3", &[true, true, false]),
    ];
    let baseline: Baseline = [
        ("clean/f05.v1".to_string(), rate(3, 3)),
        ("clean/f05.v3".to_string(), rate(3, 3)),
    ]
    .into();
    let pricing = Pricing {
        model: "claude-sonnet-4-6".into(),
        input_usd_per_mtok: 3.0,
        output_usd_per_mtok: 15.0,
    };

    let report = render(&outcomes, &baseline, &pricing);

    assert!(
        report.contains("## clean (0/1 cases pass every run)"),
        "{report}"
    );
    assert!(
        report.contains("v1                               1/3  REGRESSED from 3/3"),
        "{report}"
    );
    assert!(
        report.contains("v2                               3/3  new"),
        "{report}"
    );
    assert!(
        report.contains("v3                               2/3  dipped from 3/3"),
        "{report}"
    );
    assert!(report.contains("run 2:\n      MISS  s1"), "{report}");
    assert!(report.contains("**Regressed:** clean/f05.v1\n"), "{report}");
    assert!(report.contains("about $40.50 ($13.50 per pass"), "{report}");
}
