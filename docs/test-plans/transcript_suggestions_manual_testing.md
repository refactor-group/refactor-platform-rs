# Manual Test Plan: Suggested Actions from Transcripts

Verifies that a completed session transcript produces reviewable suggested Actions, that coach and coachee can review them under the permission rules, that failures are handled, and that no transcript text reaches the backend log. Written for a human or an AI agent with a browser and `curl`.

## Prerequisites

- The local Recall setup in `docs/setup.md` section 4 works end to end (see `docs/test-plans/local_recall_webhook_tunnel_manual_testing.md`, Part B): you can record a real Google Meet session and see its transcript complete.
- `ASSEMBLY_AI_API_KEY` is set in `.env` (section 5) and the backend was restarted. Optionally add the `cost_pricing_config` row from the production checklist to see a cost row.
- A coach and coachee in one coaching relationship that you can sign in as, plus a second Google account (or person) to join as the coachee.
- Resend configured if you want to see assignment emails (optional).
- A coaching session scheduled today, and a second session scheduled after it, so "next session" resolves. The examples assume the session is on 2026-10-12 (a Monday) and the next one on 2026-10-19.
- Create one Action on the session beforehand with the body "Send the Q4 hiring plan to Dana", so Case A has an existing action to repeat.
- Session id in `$SESSION`, and a session cookie for each user from signing in (`$COACH_COOKIE`, `$COACHEE_COOKIE`). Requests use `-H "x-version: 1.0.0"`.

List suggestions with:

```bash
curl -s -H "x-version: 1.0.0" -b "$COACH_COOKIE" \
  "http://localhost:4000/coaching_sessions/$SESSION/transcript_suggestions" | jq .
```

## Part A: extraction

### A1. Record and extract
1. Start the backend (`scripts/run_backend.sh`) and the webhook tunnel, and record the session through the tunnel. During the call say, in this order:
   - Coachee: "I'll send the revised budget to Priya by Friday."
   - Coachee: "I'm going to book a one-on-one with my manager."
   - Coach: "I'll share the delegation worksheet with you this week."
   - Coach: "Let's both think about what a good week looks like before we meet again."
   - Coachee: "Maybe I could start a reading group someday."
   - Coachee: "And I still need to send the Q4 hiring plan to Dana."
2. End the meeting and wait for the transcript to complete (`transcript.done` in the backend log).
3. Within about a minute, list the suggestions as above.

**Pass:**
- the list contains: the budget action (assignee: coachee) with `due_by` on Friday 2026-10-16 at noon in the coach's timezone and `due_phrase` "by Friday"; the one-on-one action (coachee, no `due_by`); the worksheet action (coach); the "both think" action with both the coach and coachee as assignees (one suggestion, not two);
- the "Maybe I could start a reading group someday" line produced no suggestion;
- the hiring plan suggestion has `duplicate_of` set to the existing action's id and body;
- each suggestion has a `quote` that appears in the transcript and a `segment` with a start time and speaker;
- `default_due` is the next session's start (2026-10-19), source `next_session`;
- the backend log shows `[transcript-suggestions]` and `LLM Gateway completion succeeded` lines with ids and token counts but none of the spoken words;
- in Postgres, the `transcript_analyses` row for the transcription has status `completed`, the model name, and non-null `input_tokens` and `output_tokens`:

```sql
SELECT status, attempts, model, input_tokens, output_tokens, error_message
FROM refactor_platform.transcript_analyses ORDER BY created_at DESC LIMIT 1;
```

## Part B: review as the coachee

Use the suggestions from A1, signed in as the coachee (`$COACHEE_COOKIE`).

### B1. Coach-only suggestion
1. Try to accept the worksheet suggestion (coach only): `POST .../transcript_suggestions/<id>/accept`.
2. Try to edit it: `PUT .../transcript_suggestions/<id>` with its body, due date and assignees.

**Pass:** both return 403, and the suggestion is still pending.

### B2. Adding the coach
1. Edit the one-on-one suggestion (coachee only) so `assignee_ids` includes the coach.

**Pass:** 403, and the suggestion is unchanged.

### B3. Accepting their own
1. Accept the budget suggestion.
2. Accept the same suggestion again.

**Pass:** the first returns 201 with one new Action owned by the coachee, assigned to the coachee, due 2026-10-16; the Action appears in the session's actions; if Resend is configured, an assignment email is sent. The second returns 409 and no second Action exists. The suggestion no longer appears in the pending list.

### B4. Dismiss and restore
1. Dismiss the one-on-one suggestion (`POST .../dismiss`), then list.
2. Restore it (`POST .../restore`), then list.

**Pass:** dismiss returns 204 and the suggestion leaves the list; restore returns 204 and it is back, pending. Dismissing an already dismissed suggestion returns 409.

## Part C: review as the coach

Sign in as the coach (`$COACH_COOKIE`).

### C1. Edit then accept
1. Edit the worksheet suggestion: change the body, set `due_by` to `2026-10-14T12:00:00Z`, and add the coachee as a second assignee.
2. List and check the suggestion shows the new values (and no `due_phrase` for the changed date).
3. Accept it.

**Pass:** the edit returns 200 and the list reflects it; accept returns 201 and the new Action has the edited body, the edited due date and both assignees. The coach is its owner.

### C2. Default due date
1. Accept the one-on-one suggestion (no due date) while the next session (2026-10-19) is in the future.
2. Delete or move the next session so none is scheduled after this one, then accept a suggestion that has no due date (restore one first if needed).

**Pass:** the first Action's due date is the next session's start; the second is 7 days from the moment you accepted (`default_due.source` was `seven_days` in the list).

## Part D: reliability

### D1. Bad key fails at once
1. Set `ASSEMBLY_AI_API_KEY=not-a-real-key`, restart, and complete another recorded transcript.
2. Watch the backend log for a minute.

**Pass:** the analysis fails on the first attempt (an invalid key is not retried) with `error_message` of `provider error: Authentication`; no suggestions exist; the backend log shows the failure with the analysis id and status only, with no transcript text.

### D2. Network down
1. Restore the key. Disconnect the machine from the network after the transcript completes but before the sweep runs (or point `ASSEMBLY_AI_LLM_BASE_URL` at an unreachable host and restart).
2. Watch `transcript_analyses` for about two minutes.

**Pass:** `attempts` rises to 3 over successive 30 second ticks with status returning to `pending` between attempts, then the status becomes `failed` with `error_message` of `provider error: Network` (or `Timeout`). Reconnecting does not revive a failed analysis.

### D3. No key
1. Remove `ASSEMBLY_AI_API_KEY` from `.env`, restart, and complete another recorded transcript.

**Pass:** the transcript completes and downloads normally; no `transcript_analyses` row is created for it; the list endpoint returns an empty list for that session; the log has no `[transcript-suggestions]` activity.

## Part E: privacy

### E1. No spoken text in the log
1. After Part A, pick a distinctive phrase spoken in the call, for example "delegation worksheet".
2. Search the backend log for it: `grep -ci "delegation worksheet" <backend log>`.
3. Also check `transcript_analyses.error_message` for the failed analyses from Part D.

**Pass:** the phrase does not appear in the log, and `error_message` holds only an error kind.

## Results

| Case | Result | Date | Notes |
|---|---|---|---|
| A1 |  |  |  |
| B1 |  |  |  |
| B2 |  |  |  |
| B3 |  |  |  |
| B4 |  |  |  |
| C1 |  |  |  |
| C2 |  |  |  |
| D1 |  |  |  |
| D2 |  |  |  |
| D3 |  |  |  |
| E1 |  |  |  |
