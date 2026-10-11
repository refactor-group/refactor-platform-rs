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
   - Coachee: "And I'll send the Q4 hiring plan to Dana." (A bare "I still need to..." is not a commitment; see F5.)
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

## Part F: comprehensive prompt testing

Checks what the model suggests across every kind of spoken commitment, and every look-alike, that the
prompt is meant to handle. Each group below is one short recording (about 3 to 6 minutes) between
**Jim** (coach) and **Caleb** (coachee) in their coaching relationship. Read the script naturally;
small wording changes are fine as long as the meaning stays the same. Lines in *italics* are stage
directions, not spoken.

### How to run a group

1. Do the group's **Setup** in the app (existing actions, next session).
2. Start the backend from the branch and the webhook tunnel, start recording, and read the **Script**.
3. After `transcript.done`, wait about a minute and list the suggestions (as in the Prerequisites, with
   Jim's or Caleb's cookie).
4. Compare against the **Expected** table. "None" means the line must not produce a suggestion.
5. Record the outcome in the Part F results table, noting any line that differed.

**Date notation.** `D` is the session's date in Jim's timezone, `N` the next scheduled session's date.
"Fri+" means the first Friday after `D`. Dates show as `due_by` at 12:00 noon in Jim's timezone. Groups
F6 and F7 are written for a session recorded on a **Wednesday**. Their tables say what changes on other
days.

**Every group also passes these checks:**
- every `quote` is text that appears in the stated line of the transcript (or the whole line);
- a commitment repeated or recapped appears once;
- the `transcript_analyses` row is `completed` and the backend log has no spoken words.

### F1. Plain commitments

**Setup:** next session `N` scheduled. No existing actions.

**Script:**
1. Caleb: "I'll send the revised budget to Priya by Friday."
2. Caleb: "I will update the team wiki with the new process."
3. Caleb: "I'm going to book a one-on-one with my manager."
4. Caleb: "I'm gonna clean up the shared drive."
5. Caleb: "My plan this week is to finish the vendor comparison."
6. Caleb: "I'm committing to leaving work by six every day."
7. Caleb: "Let me check with HR on the headcount numbers and get back to you."
8. Jim: "I'll send you that article on feedback after our call."
9. Jim: "I'll find you a template for the project charter."

**Expected:**

| Line | Suggestion | Assignees | Due |
|---|---|---|---|
| 1 | Send the revised budget to Priya | Caleb | Fri+ (phrase "by Friday") |
| 2 | Update the team wiki with the new process | Caleb | none |
| 3 | Book a one-on-one with your manager | Caleb | none |
| 4 | Clean up the shared drive | Caleb | none |
| 5 | Finish the vendor comparison | Caleb | blank or end of this week; phrase "this week" |
| 6 | Leave work by six every day | Caleb | none |
| 7 | Check with HR on headcount numbers and report back | Caleb | none |
| 8 | Send Caleb the article on feedback | Jim | blank or `D`; phrase "after our call" if present |
| 9 | Find Caleb a template for the project charter | Jim | none |

### F2. Requests and agreement

**Setup:** next session `N` scheduled. No existing actions.

**Script:**
1. Jim: "Could you block two hours on your calendar each week for interview feedback?"
2. Caleb: "Yes, I'll do that starting Monday."
3. Jim: "For next time, try giving feedback within a day of each interview."
4. Caleb: "Okay."
5. Jim: "Would you be willing to mentor the two new team leads this month?"
6. Caleb: "I don't think I can take that on right now, honestly."
7. Jim: "That's fair."
8. Caleb: "Could you introduce me to Maria in finance?"
9. Jim: "Sure, I can do that this week."
10. Jim: "It might help to keep a decision log."
11. Caleb: "Hmm. Maybe."

**Expected:**

| Line | Suggestion | Assignees | Due |
|---|---|---|---|
| 1-2 | Block two hours each week for interview feedback | Caleb | none ("starting Monday" is a start, not a deadline) |
| 3-4 | Give interview feedback within a day of each interview | Caleb | blank or `N` |
| 5-7 | None (declined) | | |
| 8-9 | Introduce Caleb to Maria in finance | Jim | blank; phrase "this week" |
| 10-11 | None (advice not taken up) | | |

### F3. Shapes of a commitment

**Setup:** next session `N` scheduled. No existing actions.

**Script:**
1. Jim: "Let's both think about what psychological safety looks like on your team before next time."
2. Caleb: "Yes, let's."
3. Jim: "We'll each bring one example of a hard conversation that went well."
4. Jim: "For the offsite, you'll draft the agenda and I'll bring the facilitation guide."
5. Caleb: "If the reorg is announced this week, I'll talk to HR about my role."
6. Caleb: "Every morning I'll write down my top three priorities, for the next two weeks."
7. Caleb: "For the board deck, I'll draft the outline, get Lena's feedback, and then send it to the CFO by October 30th."
8. Caleb: "And I'll ask Priya to take over the vendor reviews."
9. Jim: "So Caleb, you're going to share the retro notes with the team by Friday, right?"
10. Caleb: "Right."
11. Caleb: "I'll try to say no to at least one meeting request this week."
12. Caleb: "Should I just email the recruiter tonight about the backlog? Yeah, I'll do that."

**Expected:**

| Line | Suggestion | Assignees | Due |
|---|---|---|---|
| 1-2 | Think about psychological safety on Caleb's team | **Jim and Caleb, one suggestion** | `N` (phrase "before next time") |
| 3 | Bring an example of a hard conversation that went well | **Jim and Caleb, one suggestion** | none |
| 4 | Draft the offsite agenda | Caleb | none |
| 4 | Bring the facilitation guide to the offsite | Jim (a second suggestion) | none |
| 5 | If the reorg is announced this week, talk to HR about your role | Caleb (condition kept in the body) | blank or end of this week |
| 6 | Each morning, write down your top three priorities (frequency kept) | Caleb | blank or `D` + 14 days |
| 7 | One suggestion with steps as a bullet list (draft outline, get Lena's feedback, send to the CFO) | Caleb | October 30 |
| 8 | Ask Priya to take over the vendor reviews | Caleb | none |
| 9-10 | Share the retro notes with the team | Caleb | Fri+ |
| 11 | Say no to at least one meeting request this week ("I'll try" counts) | Caleb | blank; phrase "this week" |
| 12 | Email the recruiter about the backlog | Caleb | `D` (phrase "tonight") |

### F4. Commitments that change during the conversation

**Setup:** next session `N` scheduled. No existing actions.

**Script:**
1. Caleb: "I'll rewrite the job description by Friday. Actually, make that Monday, Friday is packed."
2. Jim: "How many of the open reqs can you review before we meet again? All five?"
3. Caleb: "Let's say two."
4. Jim: "Two it is."
5. Caleb: "Okay, two."
6. Caleb: "I'll cancel the Thursday sync. Actually no, let's keep it, the team needs it."
7. Caleb: "I'll talk to Dana about the on-call rotation."
8. *(a minute of other conversation)*
9. Jim: "Let's recap. You're rewriting the job description by Monday, reviewing two reqs, and talking to Dana."
10. Caleb: "Yes, all three."
11. Caleb: "Oh, and the job description, I'll definitely get that done Monday."

**Expected:**

| Line | Suggestion | Assignees | Due |
|---|---|---|---|
| 1 (and 9, 11) | Rewrite the job description, **once** | Caleb | first Monday after `D` (the corrected day) |
| 2-5 (and 9) | Review two of the open reqs (narrowed, not five), **once** | Caleb | `N` |
| 6 | None (retracted) | | |
| 7 (and 9) | Talk to Dana about the on-call rotation, **once** | Caleb | none |

### F5. Look-alikes that must not become actions

**Setup:** next session `N` scheduled. No existing actions.

**Script:**
1. Jim: "Let me share my screen so we can look at your goals."
2. Caleb: "I really should exercise more. I'd love to get back to running someday."
3. Caleb: "Maybe I could start a reading group. I might ask around."
4. Caleb: "And I still need to send the quarterly numbers to finance."
5. Caleb: "I already sent the offsite agenda last week, so that's done."
6. Caleb: "The quarterly plan is about halfway done."
7. Jim: "Let's always start our sessions with one win from your week."
8. Caleb: "I like that, let's do that."
9. Jim: "Want to practice? Pretend I'm your director. Go."
10. Caleb: "Okay. 'I'll have the revised forecast on your desk Friday, and I'll own the hiring plan.'"
11. Jim: "Good. How did that feel?"
12. Caleb: "My boss said, 'I'll fix the budget issue myself,' so that's on him."
13. Caleb: "My manager will send me the reorg list next week."
14. Caleb: "I'm off to Denver next week for my sister's wedding."
15. Caleb: "Sure, I'll just quit my job and become a llama farmer, ha."
16. Caleb: "I'll write that down right now so I don't lose it."
17. Caleb: "I should call the vendor, and I will, by Friday."
18. Jim: "Same time in two weeks?"
19. Caleb: "Yes. I'll stop the recording now."

**Expected:**

| Line | Suggestion | Why |
|---|---|---|
| 1 | None | done during the session |
| 2 | None | "should" and a wish without a commitment |
| 3 | None | maybes |
| 4 | None | "still need to" without "I will" |
| 5 | None | already done |
| 6 | None | progress report only |
| 7-8 | None | a working agreement, not an action |
| 9-11 | None | spoken in role-play |
| 12 | None | quoting someone else |
| 13 | None | someone else's commitment |
| 14 | None | a personal plan, not a task |
| 15 | None | a joke |
| 16 | None | done during the session |
| **17** | **Call the vendor**, Caleb, due Fri+ | positive control: "should, and I will" counts |
| 18-19 | None | scheduling the next session; the recording |

Exactly one suggestion (line 17) should exist for this group.

### F6. Due dates (record on a Wednesday)

**Setup:** next session `N` scheduled. No existing actions.

**Script** (all Caleb, one commitment per line):
1. "I'll send the status update today."
2. "I'll email the recruiter tonight."
3. "I'll get you the numbers by end of day."
4. "I'll fix the dashboard tomorrow."
5. "I'll send Maria the contract Monday."
6. "I'll finish the slides this Friday."
7. "I'll book the offsite venue by next Friday."
8. "I'll call the landlord next Monday."
9. "I'll finish the security review in two weeks."
10. "I'll get the pricing memo out two weeks from this Wednesday."
11. "I'll send the draft a week from Friday."
12. "I'll update the interview rubric by the end of the week."
13. "I'll finish the hiring plan by the end of next week."
14. "I'll read that leadership book this weekend."
15. "I'll close out the expense reports by the end of the month."
16. "I'll submit the proposal by November 20th."
17. "I'll file the form by 11/18."
18. "I'll prepare three questions before our next session."
19. "I'll reach out to the new VP soon."
20. "I'll fix the onboarding checklist ASAP."
21. "I'll set up the mentoring program at some point."
22. "I'll look into the training budget next week."
23. Jim: "I'll send you that article feedback after our call."
24. "I'll send the recap later today."

**Expected** (`D` is the Wednesday):

| Line | `due_by` | `due_phrase` | On another weekday |
|---|---|---|---|
| 1-3 | `D` | today / tonight / end of day | same |
| 4 | `D` + 1 | tomorrow | same |
| 5 | `D` + 5 (Monday) | Monday | the first Monday after `D` |
| 6 | `D` + 2 (Friday) | this Friday | the first Friday after `D` |
| 7 | **blank** | next Friday | blank when Friday is still ahead in `D`'s week, otherwise the first Friday after `D` |
| 8 | `D` + 5 (Monday) | next Monday | the same rule as line 7, for Monday |
| 9 | `D` + 14 | in two weeks | same |
| 10 | `D` + 14 | two weeks from this Wednesday | 14 days after the first Wednesday on or after `D` |
| 11 | `D` + 9 | a week from Friday | 7 days after the first Friday on or after `D` |
| 12 | `D` + 2 (Friday) | end of the week | the first Friday on or after `D` |
| 13 | `D` + 9 | end of next week | that Friday + 7 |
| 14 | `D` + 4 (Sunday) | this weekend | the first Sunday on or after `D` |
| 15 | last day of `D`'s month | end of the month | same |
| 16 | November 20 | November 20th | next year's if already past |
| 17 | November 18 | 11/18 | next year's if already past |
| 18 | `N` | before our next session | same |
| 19-22 | **blank** | soon / ASAP / at some point / next week | same |
| 23-24 | `D` | after our call / later today | same |

Deadlines on `D` itself (lines 1-3, 23-24) are due at the **end** of that day (23:59 in the coach's
timezone), never before the session; every other date is due at noon. Check `due_by` in the database
for line 23: its time must be later than the session's start.

### F7. No next session scheduled

**Setup:** make sure **no** session is scheduled after this one in Jim and Caleb's relationship. No
existing actions.

**Script:**
1. Caleb: "Before our next session, I'll map out my stakeholders."
2. Jim: "By next time, I'll send you the stakeholder template."
3. Caleb: "I'll have the draft done before we meet again."

**Expected:** three suggestions (Caleb, Jim, Caleb), each with **blank** `due_by` and its phrase kept.
The list's `default_due.source` is `seven_days`. Accepting one without a date gives a due date 7 days
from the moment of acceptance.

4. Jim: "I'll find you a template for the project chart" (pause) "and I'll get it to you by our next
   session."

**Expected:** a fourth suggestion (Jim), sourced to the first line, with **blank** `due_by` but
`due_phrase` "by our next session" kept (not null). On the card the date is the regular-weight default
(a week from today) and its tooltip quotes the phrase, so it never says "No date mentioned". Then
schedule a next session and refresh: the default becomes that session's date.

### F8. Duplicates of existing actions

**Setup:** next session `N` scheduled. Before recording, create two actions on the session:
"Send the Q4 hiring plan to Dana" and "Update the onboarding doc".

**Script:**
1. Caleb: "I'll get the Q4 hiring plan over to Dana."
2. Caleb: "Oh, and I really do need to finish updating the onboarding doc, I'll get it done this week."
3. Caleb: "I'll also write a separate onboarding checklist for interns."

**Expected:**

| Line | Suggestion | `duplicate_of` |
|---|---|---|
| 1 | Send the Q4 hiring plan to Dana | the "Send the Q4 hiring plan to Dana" action |
| 2 | Update the onboarding doc | the "Update the onboarding doc" action |
| 3 | Write an onboarding checklist for interns | none (related, but a different task) |

Then delete the "Update the onboarding doc" action and list again: line 2's suggestion stays, with
`duplicate_of` now null.

### F9. Speech quirks

**Setup:** next session `N` scheduled. No existing actions.

**Script:**
1. Caleb: "So I'll, um, I'll, uh, send the" *(pause two seconds)* "the escalation summary to the leadership channel by Friday."
2. Jim: "And I'll look up a couple of, sorry, go ahead." Caleb: "No, you go." Jim: "A couple of facilitation guides for you."
3. Caleb *(mumbled or fast)*: "I'll ping Dana about the on-call thing."

**Expected:**
- line 1: one suggestion "Send the escalation summary to the leadership channel", Caleb, Fri+. If
  the transcript splits the sentence across two lines, the `quote` is one line's actual text (filler
  words included) and is never a tidied rewrite;
- line 2: one suggestion "Look up facilitation guides for Caleb", Jim;
- line 3: one suggestion for Caleb, even if the transcript garbles a word (the body may reflect the
  garble).

### F10. A third person in the meeting (optional)

**Setup:** a third person joins the Meet (not Jim or Caleb), for example a colleague named Priya.

**Script:**
1. Priya: "Happy to help. I'll send Caleb the org chart after this call."
2. Caleb: "Thanks. I'll review it by Thursday."

**Expected:** no suggestion for Priya's line (only the coach and coachee get suggestions). Line 2 is
Caleb's, due the first Thursday after `D`.

### F11. A session with no commitments

**Setup:** next session `N` scheduled. No existing actions.

**Script:**
1. Jim: "How are you arriving today?"
2. Caleb: "Tired, honestly. It's been a heavy week."
3. Jim: "What's been the heaviest part?"
4. Caleb: "Feeling like I'm letting my team down. I keep saying yes to everything."
5. Jim: "Maybe we just sit with that today rather than solve it."
6. Caleb: "I'd like that. I could think about it more, but I don't want homework this week."

**Expected:** the analysis completes with **no** suggestions, the list is empty, and one
`transcript_suggestions_changed` event is sent so an open page refreshes its scan state.

### F12. Deadlines said after the commitment

**Setup:** next session `N` scheduled. No existing actions. Record on any day; `Fri`, `Mon` and `Thu`
mean the first such day after the session.

**Script** (pause briefly where shown, so the transcript splits the lines):
1. Jim: "I'll find you a template for the project chart" (pause) "and I will commit to giving you that
   by our next session."
2. Caleb: "I'll update the team wiki with the new process." Jim: "When by?" Caleb: "Let's say by
   Friday."
3. Caleb: "I'll call the vendor about the renewal. And I'll draft the hiring plan." (pause) "The hiring
   plan by Monday."
4. Caleb: "I'll prep the board slides." Jim: "And I'll review your draft by Thursday."

**Expected:**

| Line | Suggestions | `due_by` | Notes |
|---|---|---|---|
| 1 | Jim: find a template | `N` | sourced and quoted from the first line, phrase from the second, even if the transcript garbles "by" ("that our next session") |
| 2 | Caleb: update the wiki | `Fri` | sourced to Caleb's first line, not "Let's say by Friday" |
| 3 | Caleb: call the vendor, Caleb: draft the hiring plan | blank, `Mon` | the later deadline belongs only to the hiring plan |
| 4 | Caleb: prep the slides, Jim: review the draft | blank, `Thu` | Jim's deadline is not given to Caleb's action |

Watch for a deadline being invented for a commitment that had none: no suggestion here other than
those listed should have a date.

### Part F results

| Group | Result | Date | Lines that differed |
|---|---|---|---|
| F1 |  |  |  |
| F2 |  |  |  |
| F3 |  |  |  |
| F4 |  |  |  |
| F5 |  |  |  |
| F6 |  |  |  |
| F7 |  |  |  |
| F8 |  |  |  |
| F9 |  |  |  |
| F10 |  |  |  |
| F11 |  |  |  |
| F12 |  |  |  |

## Part G: seeing and starting a scan

Each case uses a session whose latest transcript completed (Part A works). Read the scan state and
start a scan with:

```bash
curl -s -H "x-version: 1.0.0" -b "$COACH_COOKIE" \
  "http://localhost:4000/coaching_sessions/$SESSION/transcript_suggestions/scan" | jq .
curl -s -i -X POST -H "x-version: 1.0.0" -b "$COACH_COOKIE" \
  "http://localhost:4000/coaching_sessions/$SESSION/transcript_suggestions/scan"
```

Inspect the session's action analysis with:

```sql
SELECT a.id, a.status, a.attempts, a.claimed_at, a.error_message, a.updated_at
FROM refactor_platform.transcript_analyses a
JOIN refactor_platform.transcriptions t ON t.id = a.transcription_id
WHERE t.coaching_session_id = '<session id>' AND a.target = 'action';
```

### G1. A completed scan is never repeated
1. After A1, GET the scan state and note the analysis row (id and `updated_at`).
2. POST a scan.
3. Run the SQL again and watch the backend log for a minute.

**Pass:** GET returns `{"status": "completed"}`; POST returns 409 with message "Suggested actions were
already made for this transcript."; the analysis is the same row with the same `updated_at`, no new row
exists, and the log shows no `[transcript-suggestions]` run or `LLM Gateway` call for it.

### G2. Rescanning a failed scan
1. Mark the analysis failed:

```sql
UPDATE refactor_platform.transcript_analyses
SET status = 'failed', attempts = 3, error_message = 'provider error: Network'
WHERE id = '<analysis id>';
```

2. GET the scan state, then POST a scan.
3. Run the SQL right away, then again after about 30 seconds, and list the suggestions.

**Pass:** GET returns `failed`; POST returns 202 with `{"status": "pending"}`; right after, the same row
is `pending` with `attempts` 0 and no `error_message` or `claimed_at`; within about 30 seconds the sweep
claims it, suggestions appear in the list (alongside any already there), and GET ends at `completed`.

### G3. A transcript that was never scanned
1. Delete the action analysis (its suggestions go with it):

```sql
DELETE FROM refactor_platform.transcript_analyses WHERE id = '<analysis id>';
```

2. GET the scan state, then POST a scan, and wait about 30 seconds.

**Pass:** GET returns `none`; POST returns 202 `pending`; the SQL shows one new action analysis for the
same transcription; the sweep processes it, suggestions appear, and GET ends at `completed`.

### G4. A scan already running
1. Repeat G2 step 1, POST a scan, then POST again at once (status `pending`).
2. Set the row to `processing` (`UPDATE ... SET status = 'processing' WHERE id = '<analysis id>';`) and
   POST again.

**Pass:** each second POST returns 409 with message "A scan is already running.", and the row is
unchanged by it.

### G5. Nothing to scan
1. Remove `ASSEMBLY_AI_API_KEY` from `.env`, restart, then GET and POST for the session.
2. Restore the key and restart. For a session with no transcript, or whose latest transcript has not
   completed, GET and POST.

**Pass:** in both cases GET returns `{"status": "unavailable"}` and POST returns 422 with message
"Transcript suggestions are not available for this session."; no analysis row is created.

### G6. Access
1. Sign in as a third local user who is not the coach or coachee of the session (`$OTHER_COOKIE`),
   then GET and POST.
2. Repeat as a member of the session's organization who is not in its coaching relationship.
3. Repeat with no cookie.

**Pass:** steps 1 and 2 return 403 for both GET and POST and the analysis row is unchanged; step 3
returns 401.

### Part G results

| Case | Result | Date | Notes |
|---|---|---|---|
| G1 |  |  |  |
| G2 |  |  |  |
| G3 |  |  |  |
| G4 |  |  |  |
| G5 |  |  |  |
| G6 |  |  |  |
