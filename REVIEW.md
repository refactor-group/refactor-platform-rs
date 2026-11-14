# Review Instructions

What to weigh when reviewing a change to this repository. Conventions live in `AGENTS.md` and `.claude/coding-standards.md`; this file does not repeat them.

## What failure looks like here

This backend holds private coaching notes, transcripts, and goals. In order of cost:

1. **The wrong user reads or writes.** A route with no `*_access` extractor, an extractor that loads a different resource than the handler acts on, or a query not scoped to the caller's relationship or organization. Check by route, not by diff: for every route added or changed in `web/src/router.rs`, name the check that guards it. A route with no check is a finding even when nothing in the diff looks wrong.
2. **Data is corrupted or stranded.** A migration that fails as the `refactor` user (a `create_type()` with no `ALTER TYPE ... OWNER TO refactor`), a migration whose `down` does not undo its `up`, several writes that must succeed together outside a transaction, or a query whose correctness depends on which rows it selects but is covered only by `MockDatabase` tests.
3. **Something is sent that cannot be unsent.** Emails, calendar invites, and recording bots reach real people. Look for a wrong recipient, a duplicate send on retry, or a send that happens before the write it announces has committed.
4. **It works locally and fails deployed.** A new env var missing from any passthrough layer listed in `.claude/CLAUDE.md`, or a `reqwest` client built without `.use_rustls_tls()`.
5. **The API contract changes silently.** A changed request or response shape, enum casing, or status code with no matching `utoipa` annotation and no mention in the PR description. The consumer is a separate repository and will not fail this repo's CI.
6. **A change happens and connected clients never hear of it.** Other writes to the same entity publish a `DomainEvent` and this one does not. Look hardest at writes that happen as a side effect: auto-linking, webhooks, background jobs. An event whose recipients are not derived from who may read the entity belongs under failure 1.
7. **Suggestion behavior changes with no new baseline.** A change to `actions_prompt.md`, the model, or the verification or due-date code under `domain/src/transcript_suggestion/` needs the eval rerun and an updated `baseline.json` in the same PR. Its absence is the finding.
8. **It is slow at real size.** A query per row inside a loop where one batched query would do, a list read with no bound, or a pooled connection held across a call to an external service. Name the loop and what makes it grow.
9. **Time is computed in the wrong zone.** Recurrence expansion, reminders, and due dates that mix a user's wall-clock time with UTC, or that break across a DST transition or on days 29 to 31. A change to date math with no test that crosses a DST boundary is a finding.
10. **A handler is not safe to run twice.** Webhooks are redelivered and jobs are retried. A second run must not create a second row, send a second email, or dispatch a second bot.

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
- The contents of `domain/src/transcript_suggestion/eval/fixtures/baseline.json` (whether it was updated still matters, see failure 7)

## Summary

Open with a count: "2 important, 1 nit." When there is nothing to fix, say "Nothing to fix." and stop. Then one line on what this review did not verify, such as behavior against a live Postgres.

## Rounds

- After the first pass, review only the new diff.
- Flag a class of bug once and list every instance, so one fix round covers all of them.
- Never repeat a finding that was fixed or answered.
- After three rounds with an Important finding on one PR, ask the author once to reconsider the approach, and keep reviewing new diffs.
