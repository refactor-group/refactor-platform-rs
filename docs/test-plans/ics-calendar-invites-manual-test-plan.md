# Manual Test Plan — `.ics` Calendar Invites (Phases 0-4)

Covers what is implemented so far: a `.ics` calendar invite attached to the **create**
emails for a single coaching session and for a recurring series. Reschedule, cancel, and
per-occurrence flows are NOT implemented yet (Phases 5-8), so do not expect them.

Related: [implementation plan](../implementation-plans/ics-calendar-invites-coaching-sessions.md),
issue #333.

---

## What "done so far" means (expected behavior)

When a **single** session is created, both coach and coachee receive the existing
scheduled email, now carrying an attachment `invite.ics`:
- `METHOD:REQUEST`, `STATUS:CONFIRMED`, `SEQUENCE:0`.
- `UID:<session_id>@myrefactor.com` (stable per session).
- `SUMMARY:Coaching Session: <organization name>`.
- `DTSTART;TZID=<coach zone>:<local wall time>` + a spliced `VTIMEZONE` block (when the
  coach's zone is a known IANA zone other than UTC).
- `ORGANIZER` = coach, `ATTENDEE` = coachee (`RSVP=TRUE`). Same `.ics` for both recipients.
- `LOCATION` + `URL` + `CONFERENCE` from the session `meeting_url` (the calendar "Join" button).
- `DESCRIPTION`, in order, each section omitted when empty: title line, `View this session: <url>`,
  `Topics to discuss:` + bullets, `Goals you're working toward:` + bullets,
  `Actions due for this session:` + `<body> (due <date>)` / `<body> (no due date)`.

When a **series** is created, both parties get the recurring-sessions email carrying an
`invite.ics` that is a single recurring event:
- `UID:<series_id>@myrefactor.com`, `SEQUENCE:0`, `METHOD:REQUEST`.
- `RRULE` derived from the series recurrence (`weekly` -> `FREQ=WEEKLY`; `biweekly` ->
  `FREQ=WEEKLY;INTERVAL=2`; weekdays -> `BYDAY=...`; count -> `COUNT=n`; until -> `UNTIL=...`).
- `DTSTART` anchored to the **first** session's start in the coach's zone.
- `DESCRIPTION` = `View this session: <first session url>` + the first session's in-progress
  goals only (no title/topics/actions).

The anchor timezone is the **coach's `users.timezone` at send time**.

---

## Setup / prerequisites

1. **Resend config** (real key so emails actually send): `--resend-api-key`,
   `--session-scheduled-email-template-id`, `--recurring-sessions-scheduled-email-template-id`,
   `--frontend-base-url=https://<your app>` (drives the `View this session` link).
2. **A coaching relationship** with a coach and coachee whose email inboxes you control.
   Set the **coach's timezone** to `America/New_York` for the DST/cross-zone checks.
3. Run the backend (`cargo run`) and drive it via the frontend or `curl` (create session /
   create series endpoints).

**Fastest structural smoke test without email/DB** (good for the Outlook gate): run the
throwaway spike, then import the file into Apple/Google/**Outlook**:
`cargo run -p domain --example ics_spike` -> `target/ics-spike/invite.ics` (a hardcoded
America/New_York single-session sample; proves the VTIMEZONE splice renders in your clients).

## How to inspect a received `.ics`

Save the `invite.ics` attachment and open it in a text editor. Confirm the fields listed
above. Watch for: exactly one `BEGIN:VTIMEZONE` (before `BEGIN:VEVENT`), `DTSTART;TZID=...`
(not a floating time and not `...Z`) for a known coach zone, and CRLF line endings.

---

## Happy paths

### H1 — Single session, known coach zone (the core case)
1. Coach zone `America/New_York`. Create a single session for **2026-09-15 3:00 PM ET**
   (a DST/EDT date on purpose), with a `meeting_url`.
2. **Expect:** both coach and coachee receive the scheduled email with `invite.ics`.
3. Open the `.ics`: `UID:<session_id>@myrefactor.com`, `METHOD:REQUEST`, `SEQUENCE:0`,
   `SUMMARY:Coaching Session: <org>`, one `BEGIN:VTIMEZONE` with `TZID:America/New_York`,
   `DTSTART;TZID=America/New_York:20260915T150000`.
4. Import: event lands at **3:00 PM Eastern**, organizer = coach, attendee = coachee, the
   Join button opens `meeting_url`, and the description has a working `View this session` link.

### H2 — Cross-timezone rendering (DST-aware)
1. Same session as H1. Coachee's device/calendar set to `America/Los_Angeles`.
2. **Expect:** the coachee sees the event at **12:00 PM PT** (3 PM ET). Switching a device to
   `America/Chicago` re-renders it at **2:00 PM CT** — always anchored to the coach's NY time.

### H3 — Recurring series (weekly)
1. Create a weekly series (coach `America/New_York`), several occurrences, with `meeting_url`.
2. **Expect:** one email per recipient with `invite.ics` containing `UID:<series_id>@myrefactor.com`,
   `RRULE:FREQ=WEEKLY`, `SEQUENCE:0`, `DTSTART;TZID=America/New_York:...` at the first session.
3. Import: the full series materializes; each occurrence handles DST correctly in NY.
4. Description shows `View this session: <first session url>` + the first session's in-progress
   goals only (no title/topics/actions).

### H4 — Biweekly series
1. Create a biweekly series.
2. **Expect:** `RRULE:FREQ=WEEKLY;INTERVAL=2`; occurrences land every 2 weeks.

### H5 — Rich single-session description
1. Single session WITH a title, one or more topics, at least one in-progress goal, and at
   least one open action (one with a due date, one without).
2. **Expect** in the `.ics` `DESCRIPTION`: the title line, `View this session:`,
   `Topics to discuss:` + each topic, `Goals you're working toward:` + each goal title,
   `Actions due for this session:` + `<body> (due <Mon D, YYYY>)` and `<body> (no due date)`.
   Due dates are formatted in the **coach's** timezone.

### H6 — Both recipients get the same event
1. Any created session.
2. **Expect:** coach's and coachee's `.ics` are the same VEVENT (organizer = coach, attendee
   = coachee in both), so accepting from either side references the same `UID`.

---

## Sad paths / edge cases

### S1 — Coach timezone is UTC or invalid (UTC fallback)
1. Set the coach's `users.timezone` to `UTC` (or an invalid string like `Not/AZone`).
2. Create a single session.
3. **Expect:** the `.ics` has **no** `VTIMEZONE` and **no** `TZID`; `DTSTART:<...>Z` (UTC basic).
   The event still imports (at UTC), the email still sends. For an invalid string, the server
   log shows a timezone-fallback warning; there is no crash and no missing email.

### S2 — No meeting URL
1. Create a session with `meeting_url` empty/null.
2. **Expect:** the `.ics` has no `LOCATION`/`URL`/`CONFERENCE` (no Join button), but imports fine.

### S3 — Empty description sections
1. Create a session with **no** title, **no** topics, **no** in-progress goals, **no** open actions.
2. **Expect:** the `DESCRIPTION` is just `View this session: <url>` — every empty section omitted
   (no stray headers, no blank bullet lists).

### S4 — Only completed/won't-do actions (open-actions filter)
1. Session whose actions are all `Completed` or `WontDo`.
2. **Expect:** no `Actions due for this session:` section (only not-completed actions count as open).

### S5 — Goals without titles
1. Session with in-progress goals that have `title = null`.
2. **Expect:** titleless goals are skipped in the description (only titled goals are listed); no
   blank bullets.

### S6 — Special characters
1. Org name / session title / a topic containing a comma, quote, or non-ASCII characters.
2. **Expect:** the `SUMMARY`/`DESCRIPTION` are RFC-escaped/line-folded correctly; the file still
   imports cleanly (no broken field, no split event).

### S7 — Series with a single occurrence
1. A "series" that materializes only one session (e.g. `count = 1`).
2. **Expect:** still one recurring `.ics` with `UID:<series_id>...` and an `RRULE` (e.g. `COUNT=1`);
   imports as a single event.

### S8 — Missing Resend template id
1. Start the backend without the session-scheduled (or recurring) template id configured.
2. **Expect:** creating a session/series does not crash the request; the email simply fails and
   is logged (best-effort). No partial/garbage email.

### S9 — Re-import the same invite (idempotency by UID)
1. Import H1's `.ics`, then import it again (or forward the email to yourself and re-add).
2. **Expect:** the calendar updates the existing event in place (same `UID`) rather than creating
   a duplicate.

---

## Reschedule — single session (Phase 5a)

Editing a single session now re-sends an updated invite (`METHOD:REQUEST`, **same UID**, **bumped
`SEQUENCE`**) so calendar clients update the event in place. Fires only when a calendar-relevant field
changed: `date`, `duration_minutes`, `meeting_url`, or `title`.

**Extra setup:** configure `--rescheduled-email-template-id=<your Resend template>` (single + series share
this one reschedule template). Without it, the edit still succeeds but the reschedule email fails
best-effort and is logged (see RS2).

### HR1 — Edit a session's time (happy path)
1. Create a single session (H1), import its `.ics` (note `SEQUENCE:0`).
2. Change the session's **date/time** (or duration, or `meeting_url`) via the app.
3. **Expect:** both coach and coachee receive a **reschedule** email with `invite.ics` whose `UID` is
   unchanged (`<session_id>@myrefactor.com`) and `SEQUENCE:1`.
4. Import it: the **existing** calendar event moves to the new time in place (no duplicate).
5. Edit again → next email carries `SEQUENCE:2`, still same UID, updates in place again.

### HR2 — Edit the title (description change is calendar-relevant)
1. On an existing session, set or change the **title**.
2. **Expect:** a reschedule email fires; the `.ics` `DESCRIPTION` shows the new title line and `SEQUENCE`
   is bumped. (A title edit via the dedicated title field also emits the normal in-app update.)

### RS1 — No-op edit sends nothing (sad path)
1. Submit a session update that leaves `date`, `duration_minutes`, `meeting_url`, and `title` unchanged.
2. **Expect:** **no** reschedule email is sent (nothing calendar-relevant changed); `SEQUENCE` does not bump.

### RS2 — Reschedule template not configured (sad path)
1. Run the backend **without** `--rescheduled-email-template-id`, then edit a session's time.
2. **Expect:** the edit succeeds (session updates, `ical_sequence` bumps); the reschedule email fails
   best-effort and is logged. No crash, no partial email.

---

## Not testable yet (later phases — expect NOTHING to happen)

- **Editing a series** sends no reschedule invite yet (Phase 5b); single-session editing works (see HR1).
- **Deleting** a session or series sends no cancellation `.ics` (Phase 6).
- Cancelling **one occurrence** of a series (Phase 8).
- The shared reschedule/cancel Resend templates + full env passthrough are finalized in Phase 7 (the
  `--rescheduled-email-template-id` flag exists now; you supply a template to exercise HR1/HR2).

## Outlook gate (please record the result)

Outlook desktop is the strict client this feature specifically targets (it drops undefined
TZIDs and renders floating time). H1/H3 (or the `ics_spike` sample) into **Outlook desktop**
is the outstanding verification gate before this work merges. Confirm the event renders at the
coach's correct local time with the `VTIMEZONE` respected.
