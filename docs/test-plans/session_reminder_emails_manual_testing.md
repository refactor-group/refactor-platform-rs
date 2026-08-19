# Manual Test Plan: Upcoming-Session Reminder Emails

Covers the `session-reminder` background sweep: a reminder email sent to the coachee a
configurable lead time (default 24 hours) ahead of a coaching session, plus the claim
bookkeeping that keeps it exactly-once across replicas and re-arms it on a reschedule.

Related: PR #395.

## What is implemented (expected behavior)

A sweep runs every `SESSION_REMINDER_POLL_MINUTES` (default 15), starting immediately at
process start rather than one interval later. Each tick:

1. Selects sessions starting in `(now, now + lead]` whose coachee has no current claim.
2. Claims them by upserting into `coaching_session_reminders`, keyed
   `(coaching_session_id, user_id)`, storing the session's `date` in `sent_for_start`.
3. Emails the coachee for each claimed pair, then reports `processed N item(s)`.

The claim is the write itself. The unique index arbitrates between replicas, and the
`WHERE` on the `DO UPDATE` means an already-current pair is not returned, so a reminder
is not resent every tick.

Storing the session's start rather than a "sent at" timestamp is what makes a reschedule
re-arm the reminder: the moved session no longer matches its own claim and falls back
into scope with no reschedule-path code involved.

Recipients today are coachees only. The table is keyed by recipient so coaches can be
added later without migrating claimed rows.

## Config

| Variable | Default | Effect |
|---|---|---|
| `SESSION_REMINDER_EMAIL_TEMPLATE_ID` | none | Unset disables the job entirely |
| `SESSION_REMINDER_LEAD_HOURS` | `24` | How far ahead the reminder goes out, clamped to >= 1 |
| `SESSION_REMINDER_POLL_MINUTES` | `15` | Sweep cadence, clamped to >= 1 |

## Running this plan sends real email

A local backend using the dev `.env` mails real people. Point the backend at a mock
Resend and assert on the captured payload instead. `RESEND_BASE_URL` is configurable
(`service/src/config.rs`), so this needs no code change.

Save as `/tmp/mock_resend.py`:

```python
import json, http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('content-length', 0)))
        with open('/tmp/resend_capture.jsonl', 'a') as f:
            f.write(json.dumps({'path': self.path, 'body': json.loads(body or b'{}')}) + '\n')
        code = 500 if __import__('os').path.exists('/tmp/resend_fail') else 200
        self.send_response(code); self.end_headers()
        self.wfile.write(b'{"id":"mock"}' if code == 200 else b'{"error":"boom"}')
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', 4555), H).serve_forever()
```

Touching `/tmp/resend_fail` flips it to failing, which is how the sad paths below force a
delivery error. A `RESEND_API_KEY` must be present or the client refuses to build; the
mock never checks its value.

## Setup

1. Apply migrations so `coaching_session_reminders` exists.
2. Start the mock: `python3 /tmp/mock_resend.py &`
3. Start the backend on a spare port with a short lead and cadence so cases are
   observable in seconds rather than hours:

```
RESEND_BASE_URL=http://localhost:4555 \
RESEND_API_KEY=re_mock_capture_key \
SESSION_REMINDER_EMAIL_TEMPLATE_ID=tmpl_reminder_mock \
SESSION_REMINDER_LEAD_HOURS=1 \
SESSION_REMINDER_POLL_MINUTES=1 \
  ./target/debug/refactor_platform_rs -l INFO --port 4001
```

4. A coaching relationship whose coachee you can identify. Sessions are seeded directly
   in SQL below so the cases are deterministic.

Reset between cases with:

```sql
DELETE FROM refactor_platform.coaching_session_reminders;
```

---

## Happy paths

### H1. A session inside the lead window is reminded exactly once

Seed a session 30 minutes out (inside a 1 hour lead), then wait one tick.

- **Expect** log `[session-reminder] processed 1 item(s)`.
- **Expect** one capture in `/tmp/resend_capture.jsonl` addressed to the coachee, not the
  coach, with `template_id` = the configured id.
- **Expect** one row in `coaching_session_reminders` whose `user_id` is the coachee and
  whose `sent_for_start` equals the session's `date`.

### H2. It is not resent on later ticks

Leave the backend running through several more ticks after H1.

- **Expect** no further captures and no growth in the row count.
- **Expect** ticks log at debug only (`nothing due`), never `processed`.

This is the guard that matters most. Losing it means every coachee is emailed every poll
interval.

### H3. A session outside the window is left alone until it comes due

Seed a session 3 hours out with a 1 hour lead.

- **Expect** no email and no claim row while it sits outside the window.
- Restart with `SESSION_REMINDER_LEAD_HOURS=4`.
- **Expect** it is claimed and emailed on the first tick after restart, since due-ness is
  re-derived every tick rather than scheduled at booking time.

### H4. Rescheduling re-arms the reminder

After H1, move the session's `date` forward by 15 minutes (keeping it inside the window).

- **Expect** a second email for the same session on the next tick.
- **Expect** the existing row's `sent_for_start` updated to the new `date`, still one row.

### H5. A session booked inside the window is reminded on the next tick

Create a session starting 20 minutes from now.

- **Expect** it is emailed on the next tick. This is intended: one late heads-up beats
  none, even though it lands shortly after the scheduled-session email.

### H6. The first sweep runs at startup, not one interval later

Stop the backend, seed a session inside the window, restart.

- **Expect** `processed 1 item(s)` within seconds of startup, not after a full poll
  interval. Sessions that came due during a deploy or outage are still reminded.

### H7. A batch is bounded, and the remainder drains on later ticks

Exercised at the SQL level, since `MAX_PER_TICK` is 200 and seeding 200 sessions by hand
is not worth it. Run the claim statement from `docs/architecture/background_jobs.md`
with `LIMIT 2` against a set of due sessions.

- **Expect** exactly 2 claimed, with the remainder still unclaimed and picked up by a
  subsequent run.

---

## Sad paths

### S1. No template id means the job never runs

Restart with `SESSION_REMINDER_EMAIL_TEMPLATE_ID` unset.

- **Expect** startup log `SESSION_REMINDER_EMAIL_TEMPLATE_ID not set` and no
  `[session-reminder] scheduled every Ns` line.
- **Expect** no claim rows written and no captures, ever. The feature is dark, not
  broken: nothing else in the app changes behavior.

### S2. A delivery failure releases the claim and retries

`touch /tmp/resend_fail` so the mock returns 500, then seed a session inside the window.

- **Expect** log `[session-reminder] delivery failed for session <id> to user <id>`.
- **Expect** no claim row left behind. A stamped row would mean the reminder is silently
  dropped forever.
- `rm /tmp/resend_fail`, then wait a tick.
- **Expect** the reminder is sent on the retry, proving one transient Resend failure costs
  a tick rather than the whole reminder.

### S3. Deleting a session removes its claim

Claim a session (H1), then delete the session row.

- **Expect** the claim row is gone via `ON DELETE CASCADE`, with no orphan left.
- **Expect** no email is attempted for it afterwards. Nothing queued survives a
  cancellation.

### S4. Past and in-progress sessions are never reminded

Seed one session an hour in the past and one starting exactly now.

- **Expect** neither is claimed. The window is half-open on the near side (`date > now`);
  a session already under way is past reminding.

### S5. Two replicas do not double-send

Start a second backend on another port against the same database, both with a 1 minute
cadence, then seed a session inside the window.

- **Expect** exactly one email total across both processes.
- **Expect** one claim row. Whichever replica loses the `ON CONFLICT` race gets no row
  back from `RETURNING` and sends nothing.

### S6. Blank tuning values fall back instead of crashing

Restart with `SESSION_REMINDER_LEAD_HOURS=` and `SESSION_REMINDER_POLL_MINUTES=` set to
empty strings, which is what a Compose file produces for an unset host variable.

- **Expect** the process starts and schedules every 900s. Empty env is treated as unset
  (`sanitize_empty_env`), so clap falls back to its defaults rather than failing to parse
  an empty string as a number.

### S7. Zero and absurd values are clamped

Restart with `SESSION_REMINDER_LEAD_HOURS=0` and `SESSION_REMINDER_POLL_MINUTES=0`.

- **Expect** `[session-reminder] scheduled every 60s`, not a tight loop against the
  database.
- **Expect** the lead is 1 hour, not 0, so sessions cannot slip between two ticks
  unreminded.

### S8. The migration suppresses a deploy-time blast

On a database with existing sessions, apply the migration fresh.

- **Expect** every session at or inside `now + 24h` gets a claim row for its coachee, and
  sessions further out get none.
- **Expect** starting the backend afterwards emails nobody about sessions that were
  already imminent at deploy time.

### S9. The migration is reversible

Run the migration's `down`.

- **Expect** `coaching_session_reminders` is dropped cleanly, with no leftover constraint
  or index and no change to `coaching_sessions`.

---

## Automating this

H1 through H5 and S2 through S5 are all reachable from SQL seeding plus log and capture
assertions, so they automate cleanly against the mock Resend. S1, S6, and S7 require a
process restart with different env, so they suit a shell-driven matrix rather than an
in-process test. S8 and S9 need a database that has not had the migration applied.

## Last run

2026-08-19, local backend against a local Postgres with a mock Resend, 1 hour lead and
1 minute cadence. All cases passed. H7, S8, and S9 were exercised at the SQL and
migration level rather than through the running job.
