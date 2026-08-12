# Manual Test Plan: `.ics` Calendar Invites (Phases 0-11)

Covers what is implemented so far: a `.ics` calendar invite attached to the **create** emails
for a single coaching session and for a recurring series, **reschedule** invites for both (same
UID, bumped `SEQUENCE`, so the calendar event updates in place), and **cancellation** invites for
both (`METHOD:CANCEL`, so the event is removed), and **per-occurrence** edits within a series
addressed by `RECURRENCE-ID`. Reschedule emails also state the **previous** time and, for a
series, the **previous and current repeat cadence**. All planned phases are implemented.

Related: issue #333.

**Driving this suite from the frontend:** every case below is reachable through normal app
actions (create / edit / delete a session or series). See "Automating this with Playwright"
at the end for what an automated run can and cannot assert on its own.

---

## What "done so far" means (expected behavior)

When a **single** session is created, both coach and coachee receive the existing
scheduled email, now carrying an attachment `invite.ics`:
- `METHOD:REQUEST`, `STATUS:CONFIRMED`, `SEQUENCE:0`.
- `UID:<session_id>@myrefactor.com` (stable per session).
- `SUMMARY:Coaching Session: <organization name>`.
- `DTSTART;TZID=<coach zone>:<local wall time>` + a spliced `VTIMEZONE` block (when the
  coach's zone is a known IANA zone other than UTC).
- `ORGANIZER` = the platform (`hello@mail.myrefactor.com`, shown as "Refactor Coach"); `ATTENDEE`
  = **both** coach and coachee (`RSVP=TRUE`). Same `.ics` for both recipients.
  The organizer must equal the sending address or calendar clients refuse to apply later updates
  (see "Why the platform organizes" below).
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

## ⚠️ Running this plan sends real email

**A local backend using the dev `.env` mails real people.** The dev config carries live Resend
template ids, and every create, reschedule, and cancel mails **both** participants. This is not
obvious from a local run, and it has already caught someone out: a first pass through this plan
generated an estimated 60 to 90 real sends before anyone noticed.

Two ways to avoid it, in order of preference:

1. **Point the backend at a mock Resend** and assert on the captured payload. `RESEND_BASE_URL`
   is configurable (`service/src/config.rs`), so this needs no code change. Run a second backend
   on another port so normal local development is undisturbed:
   ```
   RESEND_BASE_URL=http://localhost:4555 RESEND_API_KEY=re_mock_capture_key \
     ./target/debug/refactor_platform_rs -l INFO --port 4001
   ```
   A key must be present or the client refuses to build; the mock never checks its value. The
   invite arrives base64-encoded in `attachments[0].content`. This is the right default for
   anything except the calendar-client cases.
2. **Use a relationship where you own both inboxes**, so every send lands somewhere harmless.

The calendar-client half of this plan (does an update apply in place?) genuinely requires real
delivery, so use option 2 for those cases specifically.

## Setup / prerequisites

1. **Resend config** (real key so emails actually send): `--resend-api-key`,
   `--session-scheduled-email-template-id`, `--recurring-sessions-scheduled-email-template-id`,
   `--frontend-base-url=https://<your app>` (drives the `View this session` link).
2. **A coaching relationship** with a coach and coachee whose email inboxes you control.
   Set the **coach's timezone** to `America/New_York` for the DST/cross-zone checks.
3. Run the backend (`cargo run`) and drive it via the frontend or `curl` (create session /
   create series endpoints).

**Getting a `.ics` file to import by hand** (useful for the Outlook gate): save the `invite.ics`
attachment straight out of a received email. There is no longer a standalone sample generator;
the phase-0 spike example was removed once the real builder was covered by tests.

## Why the platform organizes (read before testing updates)

`ORGANIZER` is the platform address, **not the coach**, and both humans are attendees. This is
load-bearing, not cosmetic.

Calendar clients decide whether to apply an update by **identity**, not markup. We send from
`hello@mail.myrefactor.com`; when the invite claimed the coach as organizer, the two disagreed and
Google refused every reschedule despite a matching `UID` and a higher `SEQUENCE`. It matched the
event and declined to mutate it. Verified live 2026-08-11. `ORGANIZER;SENT-BY=...` was tried and
made it worse (Gmail stopped recognising the message as an invitation); do not reintroduce it.

Two consequences to expect while testing, both correct:

- **The coach RSVPs to their own session.** They are an attendee now. Owning the event and
  receiving working updates are mutually exclusive over emailed iTIP: Google treats an account's
  own events as authoritative and rejects external rewrites outright.
- **Each person sees a one-time "Add to calendar" prompt** the first time they receive an invite
  from `hello@mail.myrefactor.com`. Once per person, not per session. Coaches now see it too.

**Sessions created before this change can never be updated or cancelled by us** - their invites
named a coach as organizer. Use a freshly created session when testing updates.

## How to inspect a received `.ics`

Save the `invite.ics` attachment and open it in a text editor. Confirm the fields listed
above. Watch for: exactly one `BEGIN:VTIMEZONE` (before `BEGIN:VEVENT`), `DTSTART;TZID=...`
(not a floating time and not `...Z`) for a known coach zone, and CRLF line endings.

---

## Happy paths

### H1: Single session, known coach zone (the core case)
1. Coach zone `America/New_York`. Create a single session for **2026-09-15 3:00 PM ET**
   (a DST/EDT date on purpose), with a `meeting_url`.
2. **Expect:** both coach and coachee receive the scheduled email with `invite.ics`.
3. Open the `.ics`: `UID:<session_id>@myrefactor.com`, `METHOD:REQUEST`, `SEQUENCE:0`,
   `SUMMARY:Coaching Session: <org>`, one `BEGIN:VTIMEZONE` with `TZID:America/New_York`,
   `DTSTART;TZID=America/New_York:20260915T150000`.
4. Import: event lands at **3:00 PM Eastern**, organizer = Refactor Coach, both Jims as guests, the
   Join button opens `meeting_url`, and the description has a working `View this session` link.

### H2: Cross-timezone rendering (DST-aware)
1. Same session as H1. Coachee's device/calendar set to `America/Los_Angeles`.
2. **Expect:** the coachee sees the event at **12:00 PM PT** (3 PM ET). Switching a device to
   `America/Chicago` re-renders it at **2:00 PM CT**, always anchored to the coach's NY time.

### H3: Recurring series (weekly)
1. Create a weekly series (coach `America/New_York`), several occurrences, with `meeting_url`.
2. **Expect:** one email per recipient with `invite.ics` containing `UID:<series_id>@myrefactor.com`,
   `RRULE:FREQ=WEEKLY`, `SEQUENCE:0`, `DTSTART;TZID=America/New_York:...` at the first session.
3. Import: the full series materializes; each occurrence handles DST correctly in NY.
4. Description shows `View this session: <first session url>` + the first session's in-progress
   goals only (no title/topics/actions).

### H4: Biweekly series
1. Create a biweekly series.
2. **Expect:** `RRULE:FREQ=WEEKLY;INTERVAL=2`; occurrences land every 2 weeks.

### H5: Rich single-session description
1. Single session WITH a title, one or more topics, at least one in-progress goal, and at
   least one open action (one with a due date, one without).
2. **Expect** in the `.ics` `DESCRIPTION`: the title line, `View this session:`,
   `Topics to discuss:` + each topic, `Goals you're working toward:` + each goal title,
   `Actions due for this session:` + `<body> (due <Mon D, YYYY>)` and `<body> (no due date)`.
   Due dates are formatted in the **coach's** timezone.

### H6: Both recipients get the same event
1. Any created session.
2. **Expect:** coach's and coachee's `.ics` are the same VEVENT (organizer = the platform, both
   coach and coachee listed as attendees
   = coachee in both), so accepting from either side references the same `UID`.

---

## Sad paths / edge cases

### S1: Coach timezone is UTC or invalid (UTC fallback)
1. Set the coach's `users.timezone` to `UTC` (or an invalid string like `Not/AZone`).
2. Create a single session.
3. **Expect:** the `.ics` has **no** `VTIMEZONE` and **no** `TZID`; `DTSTART:<...>Z` (UTC basic).
   The event still imports (at UTC), the email still sends. For an invalid string, the server
   log shows a timezone-fallback warning; there is no crash and no missing email.

### S2: No meeting URL
1. Create a session with `meeting_url` empty/null.
2. **Expect:** the `.ics` has no `LOCATION`/`URL`/`CONFERENCE` (no Join button), but imports fine.

### S3: Empty description sections
1. Create a session with **no** title, **no** topics, **no** in-progress goals, **no** open actions.
2. **Expect:** the `DESCRIPTION` is just `View this session: <url>`, every empty section omitted
   (no stray headers, no blank bullet lists).

### S4: Only completed/won't-do actions (open-actions filter)
1. Session whose actions are all `Completed` or `WontDo`.
2. **Expect:** no `Actions due for this session:` section (only not-completed actions count as open).

### S5: Goals without titles
1. Session with in-progress goals that have `title = null`.
2. **Expect:** titleless goals are skipped in the description (only titled goals are listed); no
   blank bullets.

### S6: Special characters
1. Org name / session title / a topic containing a comma, quote, or non-ASCII characters.
2. **Expect:** the `SUMMARY`/`DESCRIPTION` are RFC-escaped/line-folded correctly; the file still
   imports cleanly (no broken field, no split event).

### S7: Series with a single occurrence
1. A "series" that materializes only one session (e.g. `count = 1`).
2. **Expect:** still one recurring `.ics` with `UID:<series_id>...` and an `RRULE` (e.g. `COUNT=1`);
   imports as a single event.

### S8: Missing Resend template id
1. Start the backend without the session-scheduled (or recurring) template id configured.
2. **Expect:** creating a session/series does not crash the request; the email simply fails and
   is logged (best-effort). No partial/garbage email.

### S9: Re-import the same invite (idempotency by UID)
1. Import H1's `.ics`, then import it again (or forward the email to yourself and re-add).
2. **Expect:** the calendar updates the existing event in place (same `UID`) rather than creating
   a duplicate.

---

## Reschedule: single session (Phase 5a)

Editing a single session now re-sends an updated invite (`METHOD:REQUEST`, **same UID**, **bumped
`SEQUENCE`**) so calendar clients update the event in place. Fires only when a calendar-relevant field
changed: `date`, `duration_minutes`, `meeting_url`, or `title`.

**Extra setup:** configure `--session-rescheduled-email-template-id=<your Resend template>`. Without it, the
edit still succeeds but the reschedule email fails best-effort and is logged (see RS2).

Single and series use **separate** reschedule templates. Resend templates have no conditional syntax and the
two paths send different variables, so one template cannot render both.

### HR1: Edit a session's time (happy path)
1. Create a single session (H1), import its `.ics` (note `SEQUENCE:0`).
2. Change the session's **date/time** (or duration, or `meeting_url`) via the app.
3. **Expect:** both coach and coachee receive a **reschedule** email with `invite.ics` whose `UID` is
   unchanged (`<session_id>@myrefactor.com`) and `SEQUENCE:1`.
4. Import it: the **existing** calendar event moves to the new time in place (no duplicate).
5. Edit again → next email carries `SEQUENCE:2`, still same UID, updates in place again.

### HR2: Edit the title (description change is calendar-relevant)
1. On an existing session, set or change the **title**.
2. **Expect:** a reschedule email fires; the `.ics` `DESCRIPTION` shows the new title line and `SEQUENCE`
   is bumped. (A title edit via the dedicated title field also emits the normal in-app update.)

### RS1: No-op edit sends nothing (sad path)
1. Submit a session update that leaves `date`, `duration_minutes`, `meeting_url`, and `title` unchanged.
2. **Expect:** **no** reschedule email is sent (nothing calendar-relevant changed); `SEQUENCE` does not bump.

### RS2: Reschedule template not configured (sad path)
1. Run the backend **without** `--session-rescheduled-email-template-id`, then edit a session's time.
2. **Expect:** the edit succeeds (session updates, `ical_sequence` bumps); the reschedule email fails
   best-effort and is logged. No crash, no partial email.

---

## Reschedule: series (Phase 5b)

> 🔴 **Blocked for any series that has content.** A session carrying a note, action, or agreement
> cannot be deleted, because those tables reference `coaching_sessions.id` with
> `ON DELETE NO ACTION`. Series reschedule and cancel both bulk-delete future sessions, so both
> return **503** once real coaching has happened in the series. Tracked as
> `refactor-platform-fe#446`; **pre-existing on `main`, not introduced by this work**, and awaiting
> a cascade-or-block product decision.
>
> Practical effect on this plan: every series case below can only be exercised on a **content-free**
> series. Note that this inverts RS5's premise, which worries that a reschedule destroys notes; the
> foreign key currently makes such a series impossible to reschedule at all.

Rescheduling a series (`PUT /coaching_session_series/:id`) re-sends the recurring invite to both
participants with the **same UID** (`<series_id>@myrefactor.com`) and a **bumped `SEQUENCE`**, so the
recurring calendar event is replaced in place. Uses its own template, configured via
`--recurring-sessions-rescheduled-email-template-id`.

Like a single-session edit, the series reschedule **does** short-circuit when nothing changed: a request
whose rule matches the stored one re-materializes nothing, burns no `SEQUENCE`, and sends no invite
(see RS5).

### HR3: Move a series to a new day/time (happy path)
1. Create a weekly series (H3) and import its `.ics` (note `SEQUENCE:0` and the `RRULE`).
2. Reschedule the series to a different start day/time.
3. **Expect:** both coach and coachee receive a reschedule email whose `invite.ics` has the **same**
   `UID:<series_id>@myrefactor.com` and `SEQUENCE:1`.
4. Import it: the **existing** recurring event moves to the new schedule in place. You should have one
   recurring event, not two.
5. Reschedule again → `SEQUENCE:2`, same UID, updates in place again.

### HR4: Change the recurrence pattern
1. Reschedule an existing weekly series to **biweekly** (or change the occurrence count).
2. **Expect:** the new `.ics` carries the updated `RRULE` (e.g. `FREQ=WEEKLY;INTERVAL=2`) with the same UID
   and a bumped `SEQUENCE`; the calendar event's repeat pattern changes in place.

### HR5: Series reschedule leaves past occurrences behind (known limitation, please confirm)
1. Create a weekly series with at least one occurrence **already in the past**.
2. Reschedule the series.
3. **Expect in the app:** past sessions are still listed. A reschedule only re-materializes future sessions.
4. **Expect in the calendar:** the past occurrences **disappear**, because the replacement recurring event
   starts at the new start date. This is a known, accepted limitation of using one UID for the whole series.
   Please confirm this is acceptable behavior for now, or flag it. Per-occurrence handling is Phase 8.

### RS3: Reschedule that produces no future sessions (sad path)
1. Reschedule a series such that the new rule yields zero future occurrences.
2. **Expect:** no email is sent (there is nothing to invite anyone to), and the reschedule itself still
   succeeds. No crash, no empty invite.

### RS4: Series reschedule with no template configured (sad path)
1. Run the backend **without** `--recurring-sessions-rescheduled-email-template-id`, then reschedule a series.
2. **Expect:** the reschedule succeeds and `ical_sequence` bumps; the email fails best-effort and is logged.

### RS5: Re-submitting an unchanged series rule changes nothing (sad path)
This is the destructive case: a series reschedule deletes and recreates future sessions, so an
unnecessary one would throw away notes and goal links to rebuild an identical schedule.

1. Create a weekly series and note the **session IDs** (or just their titles/notes) and the series
   `ical_sequence`.
2. Submit the series reschedule form again with **exactly the same** start, cadence, and duration.
3. **Expect:** no reschedule email to either party; `ical_sequence` unchanged; and critically the
   **same session rows survive**: same IDs, any notes or linked goals still attached. You should
   not see sessions blink out and come back.
4. Now change something real (move the start by a day). **Expect:** the normal reschedule fires,
   proving the guard does not jam the endpoint shut.

### RS6: Rapid successive reschedules each produce a distinct SEQUENCE
A repeated `SEQUENCE` reads to a calendar client as a duplicate of an invite it already has, and
is silently dropped, so a fast sequence of edits is the case most likely to go quietly wrong.

1. Reschedule the same session (or series) several times in quick succession, without waiting for
   each email.
2. **Expect:** `SEQUENCE` advances by exactly one per applied edit, with no repeats or gaps, and
   the calendar event ends on the **last** time you set, not an earlier one.

---

## Reschedule email content (Phases 10 and 11)

Reschedule emails say what changed, not just what it changed to. These are assertions about the
**email body**, not the attachment, so they are checked by reading the received email.

### HE1: A moved single session shows the previous time
1. Reschedule a single session to a new time.
2. **Expect** in both emails: a **Previous time:** line showing the old start and a **New time:**
   line showing the new one, each rendered in **that recipient's own timezone**.

### HE2: An edit that does not move the start renders "Unchanged"
1. Edit only the **title** or **meeting URL** of a session (still calendar-relevant, so an invite
   fires).
2. **Expect:** **Previous time:** reads exactly `Unchanged`, while **New time:** still shows the
   real start. The labels are literal template text and always render, which is why the connector
   lives inside the variable.

### HE3: A new series states its cadence
1. Create a weekly series.
2. **Expect:** the email states **Repeats: Weekly**. Cross-check it against the calendar client's
   own reading of the `RRULE` (Google shows e.g. "Weekly on Monday"); the two are computed
   independently and must agree.

### HE4: A cadence-only change shows both cadences
1. Reschedule a weekly series to **biweekly**, keeping the same start.
2. **Expect:** **Previously started: Unchanged**, **Previously repeated: Weekly**, and
   **Now repeats: Every 2 weeks**. Without this, the one thing that changed would be the one
   thing not shown.

### HE5: A start-only change shows the mirror case
1. Reschedule a series to a different start, keeping the cadence.
2. **Expect:** **Previously started:** shows the real old date and **Previously repeated:** reads
   `Unchanged`. Together with HE4 this proves the two checks are independent.

### SE1: Create and cancel emails carry no reschedule wording
1. Create a session, then cancel one.
2. **Expect:** neither email mentions a previous time or a previous cadence.

---

## Cancellation (Phase 6)

Deleting a session or a series sends a `METHOD:CANCEL` invite with `STATUS:CANCELLED`, the **same UID** as
the original, and a **bumped `SEQUENCE`**, so calendar clients remove the event. The cancellation email has
**no link and no button**: the session no longer exists, so a link would 404.

The email fires **after** the delete succeeds, so a failed delete never produces a cancellation notice.

**Extra setup:** `--session-cancelled-email-template-id` and `--recurring-sessions-cancelled-email-template-id`.

### HC1: Cancel a single session (happy path)
1. Create a **future** single session (H1) and import its `.ics`.
2. Delete the session.
3. **Expect:** both coach and coachee receive a cancellation email with `invite.ics` containing
   `METHOD:CANCEL`, `STATUS:CANCELLED`, the unchanged `UID:<session_id>@myrefactor.com`, and a `SEQUENCE`
   one higher than the last invite.
4. Open it: the event is removed from (or shown as cancelled in) your calendar.
5. Confirm the email contains **no** session link or CTA button.

### HC2: Cancel a series (happy path)
1. Create a weekly series with future occurrences and import its `.ics`.
2. Delete the series.
3. **Expect:** a cancellation email stating how many upcoming sessions were cancelled, with the first and
   last dates of that range. The `.ics` carries `METHOD:CANCEL` and the same `UID:<series_id>@myrefactor.com`.
4. Open it: the whole recurring event is removed from your calendar.
5. **In the app:** any sessions that already happened are still listed, with their notes. Only future
   sessions are deleted.

### SC1: Deleting a past session sends nothing (sad path)
1. Delete a session whose date has **already passed**.
2. **Expect:** **no** cancellation email. Deleting a completed session is housekeeping, not a cancellation,
   and mailing "your session has been cancelled" about a meeting that already happened would be wrong.

### SC2: Deleting a series with no future sessions sends nothing (sad path)
1. Delete a series whose occurrences are all in the past.
2. **Expect:** the series is deleted and **no** cancellation email is sent (there is nothing upcoming to
   cancel). Past sessions survive in the app.

### SC3: Cancel template not configured (sad path)
1. Run the backend **without** the cancellation template flags, then delete a future session.
2. **Expect:** the delete succeeds; the email fails best-effort and is logged. No crash.

### SC4: Series cancel also clears past occurrences from the calendar (known limitation)
1. Cancel a series that had at least one occurrence in the past.
2. **Expect in the calendar:** the entire recurring event disappears, including occurrences that already
   happened. **Expect in the app:** those past sessions remain, with notes. Same single-UID limitation as
   HR5. Please confirm this is acceptable, or flag it.

---

## Single occurrence of a series (Phase 8)

Editing or deleting **one session that belongs to a series** now addresses that occurrence specifically,
rather than the whole recurring event. The `.ics` carries the **series** `UID` plus a `RECURRENCE-ID`
naming the occurrence's original start, and no `RRULE`.

Before Phase 8 both of these were broken: a per-occurrence cancel named a `UID` no calendar held (so
nothing happened), and a per-occurrence reschedule created a **duplicate** standalone event next to the
untouched original. Watch specifically for those two symptoms.

The emails still use the single-session templates, since from the recipient's point of view this is one
session moving or being cancelled.

### HO1: Move one occurrence out of a series (happy path)
1. Create a weekly series (H3) and import its `.ics`. Note the recurring event.
2. Change the **date/time of a single session** in that series.
3. **Expect:** a reschedule email whose `.ics` has `UID:<series_id>@myrefactor.com` (the **series** id, not
   the session id), a `RECURRENCE-ID` matching that occurrence's **original** slot, a `DTSTART` at the new
   time, and **no** `RRULE`.
4. Import it: that one occurrence moves. **Every other occurrence stays put**, and there is **no duplicate**.

### HO2: Cancel one occurrence (happy path)
1. On the same series, delete a **single future session**.
2. **Expect:** a cancellation email with `METHOD:CANCEL`, `STATUS:CANCELLED`, the series `UID`, and a
   `RECURRENCE-ID` for that occurrence.
3. Import it: that occurrence disappears; the rest of the series remains.

### HO3: RECURRENCE-ID timezone form matches DTSTART
1. Inspect any occurrence `.ics` from HO1/HO2 with a non-UTC coach zone.
2. **Expect:** `RECURRENCE-ID;TZID=<zone>:...` in the same zoned form as `DTSTART`. A mismatch here is the
   usual reason a client silently ignores an override, so the two must agree.

### SO1: Move an occurrence twice (the address must not drift)
1. Move one occurrence, import, then move the **same** occurrence again.
2. **Expect:** the second `.ics` has the **same** `RECURRENCE-ID` as the first (the original slot, not the
   intermediate one) and a higher `SEQUENCE`. The occurrence moves again in place rather than duplicating.

### SO2: Series created before this change sends nothing (sad path)
1. Only reachable with a series materialized before the `ical_recurrence_id` migration.
2. **Expect:** editing or deleting one of its occurrences sends **no** email, with a warning logged. There
   is no valid occurrence address, and guessing would remove or move the wrong instance.

---

## Not testable yet (later phases: expect NOTHING to happen)

- Nothing. All planned phases (0-11) are implemented.
- Note the remaining known limitation in HR5 / SC4: a **series-level** reschedule or cancel still replaces
  or removes the whole recurring event, including past occurrences. Phase 8 fixed per-occurrence
  operations, not that.

## Verified live (2026-08-11, real Resend to real Gmail/Google Calendar)

Driven end to end against a real database with real email delivery, coach `jim@refactorgroup.com`
(America/New_York) and coachee `james.hodapp@gmail.com` (America/Chicago):

| Flow | Coachee | Coach |
|---|---|---|
| Create | pass | pass |
| Reschedule, applied **in place** (no duplicate) | pass | pass |
| Cancel, event removed | pass | pass |

Also confirmed in that run: all six Resend templates render with no leftover placeholders;
per-recipient timezone rendering (coachee 2:00 PM Central against coach 3:00 PM Eastern for the
same meeting); `VTIMEZONE` conversion honoured by Google **and** Apple; and the `CONFERENCE`
property rendering as a real "Join with Google Meet" button.

Earlier Apple Calendar checks (file import) confirmed create, series create, the per-occurrence
`RECURRENCE-ID` move, and series cancel. File import is a weak proxy for update semantics because
it bypasses sender identity entirely, so prefer email delivery when testing updates.

A follow-up run on **2026-08-11** verified the Phase 10 and 11 email content live: `Repeats: Weekly`
on a new series (cross-checked against Google's own "Weekly on Monday"), the cadence-only case
(HE4), the start-only mirror case (HE5), and `Previous time` / `New time` on a single-session
reschedule (HE1).

## Verified against real Postgres (2026-08-12)

The concurrency and no-op behaviors were driven through the running backend against the dev
database, since they are not observable from an email:

| Behavior | Result |
|---|---|
| Single reschedule bumps `SEQUENCE` in one transaction | 0 -> 1 |
| **10 concurrent reschedules of one session** | advanced by exactly 10, no lost updates |
| Series reschedule rewrites the rule and bumps `SEQUENCE` | 0 -> 1, biweekly spacing correct |
| Identical series rule re-submitted (RS5) | no bump, **same session rows survived** |
| A real change after a no-op | applied normally, 1 -> 2 |
| Open-actions filter with completed rows present | invite path ran clean, no enum binding error |

For contrast, the previous read-modify-write bump was replayed at the same concurrency and lost
**9 of 10** updates, which is exactly the silent-duplicate failure this feature exists to avoid.

## Outlook gate (please record the result)

Outlook desktop is the strict client this feature specifically targets (it drops undefined
TZIDs and renders floating time). Saving the `invite.ics` from an H1 or H3 email and opening it
in **Outlook desktop** is the outstanding verification gate before this work merges. Confirm the
event renders at the coach's correct local time with the `VTIMEZONE` respected.

---

## Already automated (frontend, 2026-08-12)

Three Playwright specs live in the frontend repo, 13 tests, all green against a backend built from
this branch on real Postgres. They use the mock-Resend harness described at the top, so they assert
on the invite itself rather than only on app state:

- `__tests__/e2e/ics-invite-attachment-live.spec.ts` plus `support/mock-resend.ts`: create,
  **RS1** (proven as *zero emails sent*, not merely an unchanged sequence), **HR1**, **HC1**,
  **RS6**, and **H2** (`DTSTART;TZID=America/Chicago:20270707T120000`, confirming the DST-aware
  conversion is correct rather than merely present, and the inverse UTC case emitting a bare `Z`
  with no `VTIMEZONE`).
- `ics-session-reschedule-live.spec.ts` and `ics-session-delete-live.spec.ts`: the same cases
  driven through the real UI, plus the email-failure sad path.

**RS5 passed including an edge case worth knowing:** the frontend omits `interval` when it is 1,
while the stored rule normalizes to `interval: 1`. The compare-by-meaning guard handles it; a
JSON-blob comparison would not have.

⚠️ **Known concurrency caveat, outside what RS6 asks.** With five edits genuinely in flight at
once, `SEQUENCE` still advanced correctly and without gaps, but the stored date settled on the
fourth edit rather than the fifth. Last-write-wins is not guaranteed to match last-submitted when
two people edit simultaneously. The invite is consistent with whatever the row ends up holding, so
no calendar diverges from the app; it is the row itself that picks a winner non-deterministically.

Still not covered anywhere: HE1 to HE5 and SE1 (email body copy), and every series case, blocked
by `refactor-platform-fe#446` above.

## Automating this with Playwright

The app-side half of every case automates cleanly; the calendar-side half does not. Split them.

**Drivable from the frontend and worth asserting:**
- Create / edit / delete a session and a series, including the per-occurrence cases.
- **RS5** is the highest-value automated case and needs no email at all: submit the series
  reschedule form twice with identical values and assert the session list is **byte-identical**
  (same ids, same notes) after the second submit. That regression would silently destroy user
  data, and it is invisible unless something checks.
- **RS1** likewise: a no-op session edit must not change `ical_sequence`.
- **RS6**: fire several reschedules quickly and assert the final state matches the last edit.
- Any case marked "sad path" that asserts the app still succeeds when email is misconfigured.

**Not assertable from the browser** (needs a mail inbox and a calendar client):
- Everything about the `.ics` attachment itself (`UID`, `SEQUENCE`, `RRULE`, `RECURRENCE-ID`,
  `VTIMEZONE`).
- The Phase 10/11 email-body cases HE1-HE5 and SE1.
- Whether a calendar client actually applies an update **in place** rather than duplicating,
  which is the single most important property here and depends on sender identity.

If the automated run needs to assert on invites without a real inbox, point the backend at a
mock Resend endpoint and assert on the captured payload; the attachment is base64 in
`attachments[0].content`. Do not treat a green Playwright run as covering the calendar half.
