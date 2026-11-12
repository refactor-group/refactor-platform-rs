# Colleague relationships (rs#399)

Two people hold 1:1 sessions as equals, with identical privileges. Modeled as a `kind` on the
existing coaching relationship, with a neutral `has_authority(user_id)` check, rather than a
new entity or two mirrored coaching rows.

## Today and target

- **Today:** "coach" is whoever sits in `coach_id`. That person alone can reschedule, cancel,
  run series, set meeting links and delete any topic. Many checks compare `coach_id` by hand.
- **Target:** `kind = coaching | colleague`. For coaching, `has_authority` is exactly "is the
  coach", so nothing changes. For colleagues, it is true for both people.

## Hard requirement: no regressions for coaching

Every coach and coachee permission works exactly as it does today when this is done.

| Check | When |
|---|---|
| Baseline suite (phase 0b): today's coach/coachee access as automated tests, passing on `main` before any change | Every phase. Must stay green with zero edits; review confirms `git diff main` does not touch those files. |
| `kind` defaults to `coaching`; for coaching, `has_authority` equals "is the coach" | Phase 2, pinned by a unit test |
| Phase 0c moves checks, never changes them; any diff that adds or removes a caller is sent back | Phase 0c, per handoff |
| Phase 3 switches each check to a Model method that, for coaching, admits exactly the same callers | Phase 3, per handoff |
| Manual access matrix run against the local API, all coach/coachee rows | After phase 3, and before phase 5 ships |
| Frontend companion plan's coach/coachee rows in Playwright | After phases 3 and 6 |

The only sanctioned changes to coaching behavior, each approved separately: the phase S
relationship authorization fixes (the matrix rows marked `[tightened]`), the phase M
membership rules (rows marked `[membership]`), and the phase 6 response shape change (same
access, different fields).

## Phases

Each phase is its own PR; phase 1 is two (migration and backfill, then code). Order: S, 0, M,
0b, 0c, 1, 2, 3, 3b, 4, 4b, E, 5, 6. Phases S and 0 can run in parallel. Done: S (#476), 0 (#480), M (#481).

**S. Relationship authorization fixes.** Its own security PR, ahead of the colleague work, so
the phase 0b baseline records the corrected behavior. Notes, agreements, actions, goals and
goal links require a participant of the relationship who is still an org member; action
assignees must be the relationship's coach or coachee; the batch session-goals read
authorizes every id; `POST /coaching_sessions` is coach only (colleagues later get session and
series creation through `has_authority`); series creation also requires org membership; a
coach reading a user's actions sees only their own relationship with that user. One commit per
area, each test first. The access matrix marks every changed row `[tightened]`.

**0. Capability audit** (read-only). Enumerate every route, background job and email, and who
may act or receive under coach/coachee, plus where each check lives today (`protect/`
middleware, `*_access` extractor, handler, domain, or none). Enumerate routes; never grep for
existing checks, since a route with no check is invisible to grep.

- Deliverable: `docs/test-plans/relationship_access_manual_testing.md`, a REST access matrix
  (entity × operation × caller: coach, coachee, non-participant in the same org, other org,
  admin; colleague columns added in phase 4) with curl helpers and stable row ids (e.g. `ACT-03`).
- Inventory of every coach/coachee word on the wire: params, response fields and enums
  (`role=coach|coachee`, `coach_first_name`, `coachee_actions`, transcript `speaker`, ...).
  Phase 6's contract is built from it.
- Every non-permission place that assumes coach/coachee: transcript attribution and export,
  suggested actions, email templates, background jobs.
- Settled: a coachee's Actions page (batch and per-relationship lists) shows only their own and
  unassigned actions, while every other surface shows the session's actions in full. Intended;
  recorded as is. Org admins and SuperAdmins who aren't participants list relationships but
  get 403 reading one; also intended.
- Frontend companion (written by the FE session): which controls each role sees or can use,
  each linked to its backend row id, run via Playwright.

**M. Org membership consistency.** Its own PR, before 0b. A user's own lists
(`/users/{self}/coaching-relationships`, `/coaching_sessions` with `/counts` and `/nearest`,
`/actions`) leave out relationships in orgs they no longer belong to, unless they are a
SuperAdmin, matching the 403 they get opening one. Session, series and action assignment
emails go only to participants still in the org, as reschedule and reminder emails already do.
No active-member operation involves someone who isn't a current member: creating, rescheduling
or resending a session or series is refused (409, `details.reason` `coachee_not_a_member`) when
the coachee has left, and so is assigning an action to anyone who has left
(`assignee_not_a_member`); cancelling and deleting stay allowed. Cancellations go to both
participants regardless of membership, so a removed member's calendar is cleaned up. Deleting
an account with coaching history is refused (409 `user_has_coaching_history`); removing the
person from the organization is the way out.
The access matrix marks these rows `[membership]`. The frontend confirms the rule and updates
its test plan before the PR merges.

**0b. Baseline tests.** Starts with a read-only coverage audit: each matrix row is mapped to
any existing test that already checks it (the phase S and M tests cover many), as covered,
partly covered or missing. Route rows are tested through the app's real route table
(`router::define_routes`), so phase 0c, which moves checks between middleware and extractors,
cannot touch them; an existing hand-mounted web test that checks the same row is replaced, not
kept beside it. SQLite and email recipient tests already covering a row join the frozen set
as they are. Search hits and reminder recipients run on Postgres-only SQL, so their rows are
frozen as SQL-shape mock tests plus a visible-relationship SQLite test, and checked by hand.
`PUT /actions/{id}/status` is fixed before the baseline (it answered 400 to every value).
Later handoffs only fill the gaps, and every new test is shown to fail when its rule is
broken. Every coach/coachee row of the matrix becomes a test that sends a real
request through the router as each caller and asserts status codes and returned ids (never
full bodies, so phase 6 shape changes don't touch frozen files), plus email and reminder
recipients. Rows that depend on which rows a query selects use the SQLite tier. Includes every `[tightened]` and `[membership]` row, so the phase S and M rules are locked in
as regressions too. Written against
`main` after phases S and M merge, must pass there, then frozen.

**0c. Move relationship checks onto extractors** (no behavior change; part of #218). The seven
relationship-scoped `protect/` modules (`actions`, `agreements`, `coaching_sessions`, `goals`,
`jwt`, `notes`, `users/actions`) become `*_access` extractors built on the shared
`authorize_session` / `authorize_relationship` / `authorize_relationship_coach` helpers from
phase S; `goals::by_id` gets a `GoalAccess` extractor. Each check moves exactly as it is,
including today's coach-only rule on session update, delete and resend invite. The 0b
baseline must stay green with zero edits. Then those `protect/` modules are deleted. Two
fixes ride along, each its own commit with tests: the batch session-goals check loads every
session and its relationship in one query instead of one per id; and the access helpers and
extractors answer 404 only for a missing record, so a database error is a 5xx, not a 404. About
three handoffs (list reads, sessions, goals), one PR. Afterwards every relationship check
lives in `web/src/extractors/`, so phase 3 only switches them.

**1. Record who scheduled each session** (no visible change).

- `scheduled_by_user_id` on `coaching_sessions`, set on create and reschedule; base for
  coachee-requested sessions. Null for existing sessions (either participant could create one
  and nothing recorded who); when null, the .ics timezone and meeting account fall back to the
  coach, as today.
- Record who created each meeting link (#419), backfilled only where provable.
- Per-person notice: replace `coaching_sessions.notice_given_at` with a record per (session,
  participant), stamped when that person's email goes out, backfilled from the column. The
  reminder sweep compares each recipient against their own stamp.
- Deploy order: migration plus backfill first, while old code reads the old column; then the
  reading code. Both migrations have working down migrations.

**2. Add `kind`** (no visible change). Postgres enum, default `coaching`, owned by `refactor`.
`has_authority(user_id)` on the Model. `kind` in the OpenAPI schema. Down migration.

**3. One source of truth for relationship permissions** (no visible change). Every check that
decides view/edit/delete through a relationship asks the Model:

- `grants_access_to(user)`: may see and work in the relationship
- `has_authority(user)`: may schedule, reschedule, cancel, run series, set meeting links,
  delete any topic
- `sees_all_actions(user)`: action visibility (replaces `CallerVisibility::for_relationship`'s
  hand comparison)

One handoff per check site: the extractors from phase 0c; the existing
`coaching_session_access`, `coaching_session_series_access` and `coaching_session_topic_access`
extractors; `CallerVisibility`; search visibility (`entity_api/src/search/visibility.rs`);
email recipients in `domain/src/jobs/`; and the transcription and recording controllers. Final
list from the phase 0 audit. Sites outside `web/src/extractors/` move unchanged first, then
switch to the Model method.

**3b. Finish #218.** `UserIsAdmin` (only use passes no org) is covered by `SuperAdminAccess`;
"is this my own user" becomes `UserDataAccess`. Then delete `web/src/protect/` and close #218.

**4. Colleague behavior.**

- Create: the existing `POST /organizations/{id}/coaching_relationships` takes an optional
  `kind` (default `coaching`). For colleagues, `coach_id`/`coachee_id` just name the two people.
- Same pair: a colleague relationship is refused if the pair has any relationship in the org,
  either direction. A coaching create runs the same check under the same lock and refuses only
  when the pair already holds a colleague relationship; otherwise coaching's rule is unchanged
  (A coaching B and B coaching A stays allowed).
  The rule lives in `entity_api::coaching_relationship::create`, so the invite path
  (`domain/src/user.rs`, `user_role.rs`) hits it too, and runs under
  `pg_advisory_xact_lock` on org plus the sorted pair.
- Gate: a config flag, off in production and on locally and in PR previews. While off, every
  path that would produce a colleague relationship (POST, the phase 5 switch, the invite path)
  returns 422. New env var, wired through every passthrough layer. Production turns it on in
  the phase 6 deploy. The FE admin-screen contract
  (choose kind, flip with "who becomes coach") is posted at the start of this phase.
- Both colleagues see every action; both get reminders, each by their own notice stamp.
- Colleagues only (coaching keeps using the coach): the .ics timezone comes from whoever
  scheduled. Meeting link: reuse the relationship's existing persistent link first, as today;
  if a new one is needed, the scheduler's connected account, then the other colleague's, else
  no link. The recorded link creator is whoever's account made it.
- `assignee=coach|coachee` against a colleague relationship is an error.
- Emails say "colleague" through a role-word variable in the existing Resend templates.

**4b. Transcripts and suggested actions.** Speaker attribution
(`domain/src/transcript_attribution.rs`) uses the recorded meeting-link creator and attributes
colleague speakers to people, not roles. In the transcript download, colleague speakers get a
`colleague` role; each line's `speaker_user_id` tells the two apart. Colleague sessions get suggested actions: a colleague-aware
prompt (assignees and due-date timezone no longer assume a coach), the paid eval run, and a
new `baseline.json` in which coaching results do not regress. Colleague suggestion due dates use
each assignee's own timezone; unassigned ones use the scheduler's.

**E. Start an event log.** The beginning of a real event log, printing to stdout for now. An
`AuditEvent` (actor, action from a typed enum, subject, organization, time, small details
object) and a `Recorder` trait with `record(event)`, called by domain code after a successful
write and never failing the request. Today's recorder writes one JSON line per event to stdout,
independent of log level, ids only (no names or emails), with a
fixed marker so the lines can be filtered from regular logs. Until a durable recorder exists the
record lasts only as long as production container logs; verify that retention first. It lives on `AppState` beside the SSE publisher, so a durable
recorder later is a swap, not a caller rewrite. Not built on the SSE `events` crate: those are
routing messages with no actor and only reach connected users. Scope: the trait, the stdout
recorder, an in-memory test recorder, and phase 5's kind-switch event.

**5. Change a relationship's kind.** `PUT /organizations/{org_id}/coaching_relationships/{id}/kind`,
body `{ kind, coach_id }` (`coach_id` required for coaching), authorized by
`OrganizationAdminAccess`. In place: history stays. Switching to colleague runs the same-pair
check; switching to coaching may swap the two ids. Recipients and authority are computed at
send and read time, so pending reminders, in-flight emails and future series sessions follow
the new kind; notice stamps are untouched. Switching to colleague makes the former coach a new
reminder recipient, so the switch writes suppression rows in `coaching_session_reminders` for
sessions already inside the reminder window, in the same transaction. SSE event to both people. The switch is recorded
through the phase E event log (who, when, from and to kind, new coach).

**6. Frontend contract and coordinated deploy.** Built from the phase 0 wire inventory: session
lists replace `coach` with `participants`; relationship `role` filter and
`coach_first_name`/`coachee_first_name`; `coachee_actions`; transcript `speaker`. One
coordinated FE+BE deploy, which also lifts the phase 4 gate.

Deferred to its own issue after phase 5: renaming `coach_id`/`coachee_id` (about 960
references, wire-visible), so no permission change hides in a rename diff.

## Definition of done

One end-to-end scenario, through the API and in the browser, passes in a preview with the gate
on, before the phase 6 production deploy: an
admin creates a colleague relationship; each person schedules a session; both are reminded;
both see every action; search shows both the same actions; the admin flips it to coaching;
history is intact and the new coach has authority.

## Decisions

- Name: "colleague" everywhere.
- Reminder notice: tracked per person.
- Colleague cancels: calendar update and cancel email, same as coaching; no who-cancelled record.
- #419: record who created each meeting link (phase 1).
- Session lists: replace `coach` with `participants` in one coordinated deploy.
- Suggested actions: on for colleague sessions from phase 4b.
- Email wording: a role-word variable in the existing Resend templates, no new env vars.
- Transcript download: a `colleague` speaker role value.
- Kind switches are recorded through a new event log (phase E), stdout for now.
- Same pair: no coaching plus colleague for one pair in one org.
- Columns keep `coach_id`/`coachee_id` until the separate rename.

- Kind switch: no email; open pages refresh.
- A removed member is inactive in that org; their data is kept and returns when they are
  re-added. Read-only access (#482) is a separate state: it needs its own write check beside
  `is_member`, never a reuse of it, or a read-only user would pass write checks.

- Colleague suggestion due dates: each assignee's own timezone; unassigned use the scheduler's.

All design decisions are made.

## Docs to update

`docs/architecture/action_assignees.md` (4), `email_notifications.md` (1, 4),
`background_jobs.md` (1), `transcript_suggestions.md` (4b).
