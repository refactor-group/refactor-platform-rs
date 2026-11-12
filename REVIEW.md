# Review Instructions

What to weigh when reviewing a change to this repository. Conventions live in `AGENTS.md` and `.claude/coding-standards.md`; this file does not repeat them.

## What failure looks like here

This backend holds private coaching notes, transcripts, and goals. In order of cost:

1. **The wrong user reads or writes.** A route with no `*_access` extractor or `protect` rule, an extractor that loads a different resource than the handler acts on, or a query not scoped to the caller's relationship or organization. Check by route, not by diff: for every route added or changed in `web/src/router.rs`, name the check that guards it. A route with no check is a finding even when nothing in the diff looks wrong.
2. **Data is corrupted or stranded.** A migration that fails as the `refactor` user (a `create_type()` with no `ALTER TYPE ... OWNER TO refactor`), a migration with no working `down`, several writes that must succeed together outside a transaction, or a query whose correctness depends on which rows it selects but is covered only by `MockDatabase` tests.
3. **Something is sent that cannot be unsent.** Emails, calendar invites, and recording bots reach real people. Look for a wrong recipient, a duplicate send on retry, or a send that happens before the write it announces has committed.
4. **It works locally and fails deployed.** A new env var missing from any passthrough layer listed in `.claude/CLAUDE.md`, or a `reqwest` client built without `.use_rustls_tls()`.
5. **The API contract changes silently.** A changed request or response shape, enum casing, or status code with no matching `utoipa` annotation and no mention in the PR description. The consumer is a separate repository and will not fail this repo's CI.

## Evidence

- Every finding cites `file:line` and a concrete scenario: this input or state produces this wrong result. No scenario, no finding.
- On a bug fix, confirm the change addresses the root cause and that a test fails without it.
- Compare the diff against the PR's stated intent. Code that is correct but does something the description does not mention is a finding.

## Severity

- **Important**: one of the failures above, or a correctness bug. Blocks merge until fixed or answered with a reason.
- **Nit**: at most three per review. Skip anything `cargo fmt` or `cargo clippy` already enforces.
- **Pre-existing**: not introduced by this change. Report once, never blocking.

## Skip

- `Cargo.lock`
- `docs/release-notes/`, `docs/implementation-plans/`
- `domain/src/transcript_suggestion/eval/fixtures/baseline.json`

## Summary

Open with a count: "2 important, 1 nit." When there is nothing to fix, say "Nothing to fix." and stop. Then one line on what this review did not verify, such as behavior against a live Postgres.

## Rounds

- After the first pass, review only the new diff.
- Flag a class of bug once and list every instance, so one fix round covers all of them.
- Never repeat a finding that was fixed or answered.
- After three rounds on one PR, stop and ask the author to reconsider the approach.
