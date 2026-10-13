# Suggested-Action Eval

A repeatable check of how well the real model picks suggested actions out of a transcript. It
runs text transcripts through the production prompt, the AssemblyAI LLM Gateway and the
production verification and due-date code, several times each, and scores every answer against
what the transcript should produce. Only storage is skipped.

Speech-to-text never runs: every case is text. The **clean** set is written from scripts, so a
miss there is the model's or the prompt's. The **robustness** set is real transcripts of
recordings, transcription errors included, scored separately so a garbled word never reads as a
model regression.

The code and fixtures live in `domain/src/transcript_suggestion/eval/`.

## When to run it

Run it before merging any change to:

- `domain/src/transcript_suggestion/actions_prompt.md` (the prompt);
- the model (`ASSEMBLY_AI_LLM_MODEL`) or the gateway settings;
- verification: `actions.rs`, `verify_output` in `mod.rs`, or the transcript rendering in `prompt.rs`;
- due-date resolution in `due_date.rs`.

It never runs in plain `cargo test` or in CI, because it needs the key, the network and money.
Plain `cargo test` does run its fixture and scoring tests, so a broken fixture fails CI.

## Running it

The AssemblyAI key comes from the environment or the repo's `.env`, as for the backend:

```
cargo test -p domain suggestion_eval -- --ignored --nocapture
```

A full run makes about 140 calls (47 transcripts, 3 runs each) and takes about 4 minutes. With
`claude-sonnet-4-6` it costs about $2. The report prints when it finishes and is also written to
`target/suggestion_eval/report.md`, next to each model answer
(`target/suggestion_eval/<set>/<case>.<variant>.run<N>.json`).

| Variable | Default | Meaning |
|---|---|---|
| `ASSEMBLY_AI_API_KEY` | none (required) | AssemblyAI key |
| `ASSEMBLY_AI_LLM_MODEL` | the backend's default | Model to evaluate |
| `ASSEMBLY_AI_LLM_BASE_URL` | the backend's default | LLM Gateway URL |
| `SUGGESTION_EVAL_RUNS` | `3` | Runs per transcript |
| `SUGGESTION_EVAL_CASES` | all | Comma-separated prefixes: `f06`, `clean/syn`, `robustness/` |
| `SUGGESTION_EVAL_CONCURRENCY` | `4` | Calls in flight at once |
| `SUGGESTION_EVAL_UPDATE_BASELINE` | unset | `1` writes the run's pass rates to the baseline |
| `SUGGESTION_EVAL_INPUT_USD_PER_MTOK` | `3.0` | Input price used for the cost estimate |
| `SUGGESTION_EVAL_OUTPUT_USD_PER_MTOK` | `15.0` | Output price used for the cost estimate |

Rate-limited calls wait as long as the gateway asks and are retried. When iterating on one
behavior, run only its cases, for example `SUGGESTION_EVAL_CASES=f06,syn-dates`.

## Reading the report

```
clean/f12-later-deadlines          1/3
  v1                               1/3  dipped from 3/3
    run 2:
      EXTRA s2 [Coach] due 2026-10-21 ("by our next session") | Find a project chart template for Casey
      MISS  ["s1"] deadline on the next line
  v2                               3/3
```

- A **variant** is one transcript. Its rate is the runs that produced exactly the expected
  suggestions, with nothing wrong, missing or extra.
- A **case** passes a run only when every variant passes that run, so its rate is never higher
  than its weakest variant's. A miss confined to one variant points to wording.
- Each failing run lists what went wrong:
  - `WRONG`: matched an expected suggestion, but its assignees, due date, duplicate flag or quote
    is off; the reason follows `<--`.
  - `EXTRA`: a suggestion nothing expected, such as a look-alike or a commitment counted twice.
  - `MISS`: an expected suggestion that never came.
  - `NOTE`: the action text reads like a note or placeholder (brackets, a question mark,
    "unclear", "TBD", "not specified"). Wording quality otherwise stays a human review.
  - `ERROR`: the call or verification failed.
- Against the baseline, each variant shows `new`, nothing (unchanged), `improved from`,
  `dipped from` or `REGRESSED from`.

**Regressions.** The model varies from run to run, so a drop of one run (3/3 to 2/3) is shown as
a dip and does not fail. A drop of more than one run's worth (3/3 to 1/3) is a regression and
fails the test. When a dip looks suspicious, rerun just that case with more runs:
`SUGGESTION_EVAL_CASES=f12 SUGGESTION_EVAL_RUNS=10`.

**Clean or robustness.** A miss in both the clean and robustness versions of a group points to
the model or the prompt. A miss only in a recording points to transcription.

## Updating the baseline

`fixtures/baseline.json` holds the last accepted pass rate of every variant, such as
`"clean/f06-dates.v1": "0/3"`. A rate below 3/3 records a known weakness, not a broken test.

After a deliberate change (a prompt edit, a new model, a new case), run the full suite with
`SUGGESTION_EVAL_UPDATE_BASELINE=1` and commit `baseline.json` in the same commit as the change,
so the diff shows what the change did to every case. A run with a case filter updates only the
variants it ran. Never update the baseline only to make a regression pass.

## Fixtures

Each case is one or more transcripts plus the suggestions they must produce:

```
fixtures/clean/f12-later-deadlines.v1.txt
fixtures/clean/f12-later-deadlines.v2.txt
fixtures/clean/f12-later-deadlines.expected.json
fixtures/robustness/f12-later-deadlines.rec1.txt
fixtures/robustness/f12-later-deadlines.rec1.expected.json
```

`<case>.<variant>.txt` is scored against `<case>.<variant>.expected.json` when that file exists,
and against `<case>.expected.json` otherwise.

### Transcript

```
# Any line starting with # that is not a known header is a comment.
# session: Wednesday 2026-10-07
# next_session: Wednesday 2026-10-21 10:00
# coach: Jim Hodapp
# coachee: Casey Morgan
# existing a1: Send the Q4 hiring plan to Dana
[s1 00:05] Coach: I'll find you a template for the project chart
[s2 00:10] Coach: and I will commit to giving you that by our next session.
[s3 00:18] Priya Shah: Happy to help.
```

- `session` is the session's date in the coach's timezone. `next_session` is a date and an optional
  `HH:MM` start (default 10:00), or `none`.
- Optional: `timezone` (IANA, default `America/Chicago`), `week_end` (default `Friday`), and one
  `existing <label>` line per action already on the session.
- Speakers are `Coach`, `Coachee` or any other name, which the model sees as a third person.
- The label in brackets is the line's name in the expected file. The model sees lines renumbered
  `s1..sN` in order, as in production; the eval maps its answer back to these labels.

### Expected suggestions

```json
[
  {"segments": ["s1", "s2"], "assignees": ["coach"], "due_dates": ["2026-10-21"], "duplicate_of": null, "note": "deadline on the next line"},
  {"segments": ["s4"], "assignees": [["coach", "coachee"], ["coachee"]], "due_dates": [null, "2026-10-09"], "duplicate_of": "a1", "note": "..."}
]
```

- `segments`: the lines the suggestion may be sourced from. A suggestion said once and recapped
  later lists every line it may come from, and must still appear once.
- `assignees`: one set, or a list of acceptable sets when the transcript genuinely allows both.
- `due_dates`: every acceptable date in the coach's timezone; `null` means no date. List several
  only for a genuinely ambiguous deadline.
- `duplicate_of`: the `existing` label the suggestion repeats, or `null`.
- `note`: why the item exists; shown on a miss.
- `[]` means the transcript must produce no suggestions at all.

## Adding a case

1. Write the transcript in `fixtures/clean/` as `<case>.v1.txt`. Use fictional names; the repo is
   public. Date the session so the expected dates are easy to check (the existing cases use
   Wednesday 2026-10-07, next session 2026-10-21).
2. Write `<case>.expected.json`, listing every suggestion the transcript should produce. Lines that
   must not produce one simply have no entry.
3. Write at least one more wording as `<case>.v2.txt` (below).
4. Run `cargo test -p domain transcript_suggestion::eval`. It checks that every fixture parses,
   that every expected line exists, and that every clean case has two wordings.
5. Run the eval for the new case, read the misses, and fix the fixture if the expectation was wrong.
   Then add its rates to the baseline with `SUGGESTION_EVAL_CASES=<case> SUGGESTION_EVAL_UPDATE_BASELINE=1`.

### Writing a variant

A variant says the same commitments, with the same expected results, in different words. For
example, "I still need to send the quarterly numbers to finance" next to "the numbers for finance
are still on my plate".

- Keep the line labels of the lines that matter, so both variants share one expected file. A
  variant that needs an extra line gives it a new label (`s1b`) and adds that label to the item's
  `segments`.
- Change sentence shape and vocabulary, not meaning. For date cases, keep the deadline phrase and
  reword around it, since the phrase is what's being tested.
- Never copy a variant's wording into an example in `actions_prompt.md`. The suite would then
  measure memorization, not the rule.

### Turning a recording into a robustness case

1. Record the script (the Part F groups in `transcript_suggestions_manual_testing.md`) and download
   its transcript.
2. Write one fixture line per transcript segment, keeping the transcription errors exactly. Label
   lines `s1..sN` in order, and change real names to fictional ones.
3. Write `<case>.rec1.expected.json` for what this transcript, as transcribed, should produce. When
   a lost word makes a reading genuinely ambiguous, accept both readings in `assignees` or
   `due_dates`, and say so in the `note`.

**A second recording.** Record a script again only when a robustness miss could be transcription,
meaning dropped or garbled words near the miss. Add it beside the first as
`<case>.rec2.txt` with its own `rec2.expected.json`. Each recording is scored on its own: a miss in
one recording but not the other points to transcription, and a miss in both points to the model.
