//! Opt-in evaluation of suggested-action extraction against the real model.
//!
//! Text transcripts in `fixtures/` run through the production prompt, gateway and verification,
//! several times each, and are scored against their expected suggestions. Only `suggestion_eval`
//! calls the model, and it is `#[ignore]`d; the fixture and scoring tests run in plain `cargo test`.
//! How to run it and read its report: `docs/test-plans/transcript_suggestions_eval.md`.

mod case;
mod report;
mod run;
mod score;

mod case_tests;
mod report_tests;
mod score_tests;
