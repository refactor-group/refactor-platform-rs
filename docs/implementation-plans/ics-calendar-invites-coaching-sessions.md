# Attach `.ics` Calendar Invites to Coaching-Session Emails

Implementation plan for [issue #333](https://github.com/refactor-group/refactor-platform-rs/issues/333).

## Goal

Attach an RFC 5545 `.ics` file to the transactional emails sent for coaching-session lifecycle
events so coach and coachee can add, update, or remove the meeting in Apple Calendar, Google
Calendar, and Outlook with one click. The lifecycle spans seven scenarios across two cardinalities
(single session, series) and the actions schedule, update/reschedule, cancel, and per-occurrence cancel.

## Scope phasing at a glance

- **Foundation first:** one reusable `.ics` builder that can emit *every* scenario shape, built
  and unit-tested before anything consumes it.
- **Attach to existing emails first:** the create flows (new single, new series) already send emails;
  Phase 4 just adds the `.ics` attachment to them.
- **Net-new emails follow:** update/reschedule and cancel currently send no email. Those net-new flows
  (single, series, then per-occurrence) land in later phases, once the foundation and create-flow
  attachment are proven. They are sequential next steps, not deferred wishlist items.

---

## Dependencies

### PR #356 — `coaching_session_series` (OPEN, hard dependency)
[PR #356](https://github.com/refactor-group/refactor-platform-rs/pull/356) makes a recurring series a
first-class entity. This work **must be built on top of #356** (branch from it or land after it merges)
because they overlap heavily and #356 changes the surfaces this plan hooks into:

- New `domain::coaching_session_series` with `SeriesRule { start_at, recurrence, duration_minutes }`
  (typed JSONB) and functions `create_with_sessions`, `reschedule`, `delete_with_future_sessions`,
  `find_by_relationship`.
- New entity `coaching_session_series { id, coaching_relationship_id, rule: JSONB, created_by_user_id, … }`
  and a new `coaching_sessions.coaching_session_series_id: Option<Id>` FK (`ON DELETE SET NULL`).
- New endpoints: `POST/GET/GET:id/PUT/DELETE /coaching_session_series`. **The old
  `POST /coaching_sessions/recurring` route is removed.**
- Email hooks after #356: the series `create` controller calls `notify_recurring_sessions_scheduled`
  (existing email, **no `.ics` yet**). The series `update` (reschedule) and `delete` controllers fire
  **no** notification — those are the empty seams for future Phases 5/6.
- #356 also touches `domain/src/emails.rs`, `domain/src/coaching_session.rs`,
  `domain/src/coaching_session/recurrence.rs`, the `coaching_sessions` entity, and `migration/src/lib.rs`
  — the same files this plan edits. Treat merge ordering as a real constraint.

### #332 — `duration_minutes` (MERGED)
`duration_minutes: i16` is available on `coaching_sessions` for `DTEND` computation. `title` and the
topics tables also already exist.

---

## Codebase reconciliation (verified)
- `chrono-tz = "0.10"` is already a `domain/` dependency.
- `coaching_session_goal::find_in_progress_goals_by_coaching_session_id` exists (`coaching_session_goal.rs:274`).
- `domain::coaching_session::update(db, id, params)` does **not** take `&Config`; `delete(db, config, id)` does. The future single-update email phase must thread `&Config` into `update()` and its two call sites.
- Subjects are template-owned: never set `subject` in the Resend payload (guarded by `emails.rs` test helper).
- Status enum (`entity/src/status.rs`): open actions = `status != Completed` (confirm `WontDo` handling).
- This repo is **GPL-3.0**, so GPL-3.0-or-later crates link fine (relevant to the VTIMEZONE crate choice below).

---

## The scenarios map to one reusable builder

The whole point of the foundation phase: the scenarios are not seven builders — they are seven
*populations* of one input struct, varying along three orthogonal axes (METHOD, cardinality, sequence)
plus, for scenario 7, an `EXDATE`/`RECURRENCE-ID` exception.

| # | Scenario | Trigger (post-#356) | METHOD | Cardinality | UID | SEQUENCE | Email |
|---|---|---|---|---|---|---|---|
| 1 | Schedule single | `coaching_session::create` → `notify_session_scheduled` | `REQUEST` | single `VEVENT` | `<session_id>@…` | 0 | exists |
| 2 | Schedule series | `series::create_with_sessions` → `notify_recurring_sessions_scheduled` | `REQUEST` | `VEVENT`+`RRULE` | `<series_id>@…` | 0 | exists |
| 3 | Update single | `coaching_session::update` (calendar-relevant change) | `REQUEST` | single `VEVENT` | `<session_id>@…` | bumped | net-new (Ph 5) |
| 4 | Update series | `series::reschedule` | `REQUEST` | `VEVENT`+`RRULE` | `<series_id>@…` | bumped | net-new (Ph 5) |
| 5 | Cancel single | `coaching_session::delete` | `CANCEL` | single `VEVENT` | `<session_id>@…` | bumped | net-new (Ph 6) |
| 6 | Cancel series | `series::delete_with_future_sessions` | `CANCEL` | `VEVENT`+`RRULE` | `<series_id>@…` | bumped | net-new (Ph 6) |
| 7 | Cancel one occurrence in a series | `coaching_session::delete` on a series-linked session | `REQUEST`(+`EXDATE`) or `CANCEL`(+`RECURRENCE-ID`) | series `VEVENT`+`RRULE` | `<series_id>@…` | bumped | net-new (Ph 8) |

### Reusable builder design (`domain/src/gateway/ical.rs`)
A single entry point driven by an input struct. No DB, no email, no I/O — pure and unit-testable.

```rust
pub enum Method { Request, Cancel }
pub enum EventStatus { Confirmed, Cancelled }

pub struct IcsInvite<'a> {
    pub uid: String,                 // "<session_id|series_id>@myrefactor.com" — stable across lifetime
    pub sequence: i32,               // DB-backed (PG INTEGER -> i32; PG has no unsigned int, so signed). 0 on create; bumped on update/cancel
    pub method: Method,              // Request (create/update) | Cancel
    pub status: EventStatus,         // Confirmed | Cancelled
    pub summary: String,             // "Coaching Session: {organization_name}"
    pub description: String,         // composed sections (see below)
    pub anchor_tz: chrono_tz::Tz,    // coach's zone — drives TZID + spliced VTIMEZONE
    pub start: chrono::NaiveDateTime,// session/series start (UTC-naive in DB)
    pub duration_minutes: i16,
    pub organizer: &'a users::Model, // coach — ORGANIZER (reuse existing entity, don't reinvent)
    pub attendee: &'a users::Model,  // coachee — ATTENDEE (RSVP=TRUE)
    pub location_url: Option<String>,// meeting_url -> LOCATION + URL + CONFERENCE
    pub recurrence: Option<Recurrence>, // None = single VEVENT; Some = VEVENT + RRULE
}

pub fn build(invite: &IcsInvite) -> Result<String, Error>; // returns the full VCALENDAR text
```

`organizer`/`attendee` are `&users::Model` directly — no new `Attendee` type. The builder derives the
`CN` (display name) from the model the same way the email code already does
(`display_name` when present, else `format!("{first_name} {last_name}")`, cf. `emails.rs:365`) and the
`mailto:` from `users.email`. This keeps a single source of truth for participant identity.

- `recurrence: None` emits a single `VEVENT`; `Some(rule)` adds an `RRULE` derived from the typed
  `Recurrence` (`Biweekly` → `FREQ=WEEKLY;INTERVAL=2`; `by_weekdays` → `BYDAY`; `count` → `COUNT`;
  `until` → `UNTIL`).
- `method`/`status` set `METHOD:REQUEST|CANCEL` and `STATUS:CONFIRMED|CANCELLED`.
- The builder always splices the coach-zone `VTIMEZONE` (see below) and TZID-anchors `DTSTART`/`DTEND`.
- A small description-composer fn (also pure) builds the `DESCRIPTION` from session fields.

Because the struct already models every axis, Phases 4/5/6 add **zero** new builder code — they only
populate `IcsInvite` differently and wire the email. This is the foundation everything else stands on.

---

## `VTIMEZONE` decision (spike resolved)

A spike on `icalendar` (v0.17.x) found it emits `DTSTART;TZID=…`, `RRULE`, `SEQUENCE`, and typed
`ATTENDEE` (`RSVP=TRUE`); `METHOD`/`ORGANIZER`/`CONFERENCE`/`STATUS`-setter go through the generic
`append_property` path. But **`icalendar` cannot emit a `VTIMEZONE` component** — and without it,
RFC-strict clients (Outlook desktop) silently drop the TZID and render floating local time.

**Decision: use `vtimezones-rs` for the `VTIMEZONE` block.** This repo is GPL-3.0 and `vtimezones-rs`
is GPL-3.0-or-later — fully compatible. It exposes `VTIMEZONES: phf::Map<IANA name, &str>` where each
value is a complete, build-time-generated `VTIMEZONE` block (full `RRULE`-based DST rules; bundled TZDB
2025b). More correct and far less code than deriving transitions from `chrono-tz`.

**Integration friction (the real Phase 0 risk):** `icalendar` has no API to inject a foreign
component, so the builder serializes the `Calendar` and splices the `VTIMEZONE` string in before the
first `BEGIN:VEVENT` (which places it after the `VERSION`/`PRODID`/`METHOD` header).

**Phase 0 confirmed mechanism (resolved, commit `6ab4f587`, throwaway spike `domain/examples/ics_spike.rs`):**
Resolved crate versions `icalendar 0.17.11` + `vtimezones-rs 0.3.1`. Mechanical splice verified to emit
a single, correctly nested, CRLF-clean `VCALENDAR` with TZID-anchored `DTSTART`/`DTEND` (human
calendar-import test still pending). Two facts the real Phase 1 builder must honor:
- **TZID anchoring:** `icalendar 0.17` does NOT implement `From<DateTime<Tz>>`. Build a zoned value via
  `CalendarDateTime::from((naive_local, tz))` (the `(NaiveDateTime, Tz)` tuple impl, gated on the
  `chrono-tz` feature). This yields `CalendarDateTime::WithTimezone`, which serializes as
  `DTSTART;TZID=America/New_York:20260915T150000` (local wall time, no `Z`, not floating).
- **`vtimezones-rs` value is NOT a bare `VTIMEZONE` block (plan was wrong here):** in 0.3.1,
  `VTIMEZONES.get("America/New_York")` returns a value wrapped in a full `BEGIN:VCALENDAR … END:VCALENDAR`
  envelope. Splicing it verbatim produces a malformed file (a stray `END:VCALENDAR` before the `VEVENT`).
  The builder MUST slice out the inner `BEGIN:VTIMEZONE..END:VTIMEZONE` span (and normalize it to CRLF)
  before splicing. The spike's `extract_vtimezone` + `splice_vtimezone` helpers are the reference impl to
  port into `ical.rs`.

**Edge case:** a `users.timezone` missing from the map — define the fallback (skip-attachment-and-log,
or UTC `Z` encoding for that send). See Open Questions.

**Maintenance note:** `vtimezones-rs` pins a TZDB snapshot at its build time, not host tzdata. A DST-rule
change in any jurisdiction needs a `cargo update` of the crate to land. Add a periodic
dependency-freshness check to the release checklist.

---

## `DESCRIPTION` composition

Sections separated by blank lines; each omitted entirely when empty.

**Single session** (scenarios 1/3/5) — all sections, in order:
1. **Title header** — when `session.title` is set, a leading line with the title.
2. **In-app link** (always): `View this session: {frontend_base_url}/coaching-sessions/{session_id}`.
3. **Topics** (when any): `Topics to discuss:` + bulleted topic list.
4. **In-progress goals** (when any): `Goals you're working toward:` + bulleted titles
   (`coaching_session_goal::find_in_progress_goals_by_coaching_session_id`).
5. **Open actions** (when any): `Actions due for this session:` + `{body} (due {date})` /
   `{body} (no due date)`; due dates formatted in the **coach's TZ**. "Open" reuses the existing
   shared definition — `!Status::is_completed()` (excludes `Completed` and `WontDo`); see Phase 3.

**Series** (scenarios 2/4/6) — the invite is one shared `VEVENT`, so per-session fields don't apply:
- **In-app link** (always): points to the **first (next upcoming) session's** existing view page —
  `View this session: {frontend_base_url}/coaching-sessions/{first_session_id}` — with "session"
  wording, not "series". There is no series view route today, so linking to the first session gives a
  real, working destination, and it's the same session the goals below are drawn from. (When a series
  view route ships as a #356 follow-on, swap to the series URL + "View this series:" wording.)
- **In-progress goals of the first session** (when any): goals are the ongoing objectives worked
  across the whole series, so the first occurrence's in-progress goals represent the series.
- **Omit** title, topics, and per-session open actions — these are unique to each individual session.

No truncation cap in v1.

---

## Phased implementation

Each phase is an independently compilable, reviewable unit that **merges to `main` one at a time** —
phase N+1 branches from `main` after phase N lands. The phases are sequential in dependency, not just
in priority; each builds functionally on the ones before it. All phases assume #356 has landed.

Note on Phase 1 merging standalone: the core builder has no consumers until Phase 4, so it carries
`#[cfg_attr(not(test), allow(dead_code))]` (or equivalent) until Phase 4 wires it in. Its tests prove
it works, so it's a safe, reviewable merge on its own.

### Phase 0 — `VTIMEZONE` splice spike (BLOCKING, throwaway) — Apple+Google ✅, Outlook deferred to pre-Phase-4
Status: spike built + overseer-verified (commit `6ab4f587`); confirmed mechanism recorded under the
`VTIMEZONE decision` section above. Emitted `.ics` reproducible via `cargo run -p domain --example
ics_spike` → `target/ics-spike/invite.ics`.
Human import test (2026-06-14): **Apple Calendar and Google Calendar PASS** — event anchored to 15:00
EDT rendered correctly as 14:00 in a US Central (CDT) device on both clients (DST-aware TZID anchoring
confirmed; Apple even annotated "(15:00 EDT)"). **Outlook desktop NOT tested (no access).** Decision:
proceed to Phases 1-2 (non-user-facing, standalone-mergeable); **Outlook verification is a required gate
before Phase 4 merges** (first phase where an `.ics` reaches a real inbox).

- Scratch branch: add `icalendar = { version = "0.17", features = ["recurrence", "chrono-tz"] }` and `vtimezones-rs = "0.3"` to `domain/Cargo.toml`.
- Prototype the splice: serialize an `icalendar::Calendar`, inject `VTIMEZONES["America/New_York"]` after `PRODID`/`VERSION`.
- **Acceptance:** the emitted `VCALENDAR` (TZID-anchored `VEVENT` + spliced `VTIMEZONE`) imports cleanly into Google + Apple + **Outlook desktop** at correct local time. No production code merges until green. Record the confirmed splice mechanism in this doc.

### Phase 1 — CORE: reusable `.ics` builder (foundation) — DONE ✅ (commit `22863616`, overseer-verified)
Status: built + overseer-verified. `domain/src/gateway/ical.rs` + frozen tests in
`domain/src/gateway/ical_tests.rs` (8 tests, all pass; clippy/fmt clean). Module dead-code-gated
(`#[cfg_attr(not(test), allow(dead_code))]`) until Phase 4 wires it in. Decisions locked this phase:
(1) `IcsInvite` gained an explicit `dtstamp: NaiveDateTime` field so `build` stays pure/clock-free;
(2) the composer (`compose_description` + `DescriptionParts`/`OpenAction`) takes PLAIN data
(`title: Option<&str>`, `topics: &[String]`, `goal_titles: &[String]`, `open_actions`), NOT entity
models, because `coaching_sessions.title` and a topics entity do not exist yet on the #356 branch (the
Phase 4 caller extracts/passes the data); (3) RRULE mirrors the series materializer (Biweekly →
`FREQ=WEEKLY;INTERVAL=2`, effective interval = base × `recurrence.interval`); (4) builder output is
RFC-5545 line-folded (tests unfold before asserting logical content).
- `domain/src/gateway/ical.rs`: `IcsInvite<'a>` (organizer/attendee as `&users::Model`), `Method`, `EventStatus`, `build()`, and the pure description-composer fn. Add the two crates (lock-in from Phase 0).
- Implement single vs. recurring (`RRULE`), METHOD/STATUS, TZID anchoring + `VTIMEZONE` splice, UID/SEQUENCE pass-through, and the **UTC fallback** when the coach's zone is absent from the `VTIMEZONES` map (emit `DTSTART`/`DTEND` as UTC `Z` with no `VTIMEZONE`/TZID; log a warning).
- Derive participant `CN` from `display_name` else `first_name last_name`, and `mailto:` from `email`.
- Frozen-test discipline: tests in `domain/src/gateway/ical_tests.rs`, wired via `#[cfg(test)] #[path = "ical_tests.rs"] mod tests;`.
- **Acceptance (the six shapes as input permutations):**
  - [x] `VTIMEZONE` present with TZID == coach zone for a known zone; UID/SEQUENCE/METHOD/STATUS correct per shape.
  - [x] An unknown coach zone (and UTC) falls back to UTC `Z` encoding with no `VTIMEZONE`; warns when a non-UTC zone is absent.
  - [x] `RRULE` shape correct per `Frequency` (incl. `Biweekly` → `FREQ=WEEKLY;INTERVAL=2`).
  - [x] Single-session `DESCRIPTION` includes title/topics/goals/actions sections, each omitted when empty.
  - [x] Series `DESCRIPTION` includes only the in-app link + first-session goals; omits title/topics/actions.
  - [x] CANCEL shape emits `METHOD:CANCEL` + `STATUS:CANCELLED`.

### Phase 2 — Resend attachment plumbing — DONE ✅ (commit `ed23fdbd`, overseer-verified)
Status: built + overseer-verified. `Attachment` + `attachments: Option<Vec<Attachment>>` (omit-when-none)
+ builder `add_ics_attachment(&str, &ical::Method)` (base64 via `base64 0.22` STANDARD engine). Resend
JSON keys `filename`/`content`/`content_type` confirmed against Resend docs (snake_case, no rename). New
method dead-code-gated (`#[cfg_attr(not(test), allow(dead_code))]`) until Phase 4. 19 resend tests pass
(16 existing unchanged + 3 new); teeth-check confirmed the omit-when-none guard (removing
`skip_serializing_if` breaks both the new test and the existing full-shape regression test). clippy/fmt clean.
- `domain/src/gateway/resend.rs`: add `Attachment { filename, content_type, content }` (base64-inline) and `attachments: Option<Vec<Attachment>>` on `SendEmailRequest` (`skip_serializing_if = "Option::is_none"`).
- Builder method `add_ics_attachment(ics_body, method)`: `filename = "invite.ics"`, `content_type = "text/calendar; method=REQUEST|CANCEL; charset=UTF-8"`, base64-encoded body.
- **Acceptance:**
  - [x] `attachments` serializes with base64-encoded content and the correct `content_type` per METHOD.
  - [x] `attachments` is omitted from the payload when `None` (no regression to existing emails).

### Phase 3 — Data layer: sequence columns + open-actions query — DONE ✅ (commit `1882f83a`, overseer-verified)
Status: #356 merged 2026-06-19 (`0ffafb66`); `main` merged into `feat/ics-calendar-invites`
(`19c89215`). Phase 3 built + overseer-verified. Migration `m20260702_000000_add_ical_sequence`
(`ical_sequence INTEGER NOT NULL DEFAULT 0` on both tables, reverse-order down, registered last);
both entities gained `ical_sequence: i32` `#[serde(skip_deserializing)]`; `Status::is_completed()`
promoted (single `matches!(Completed|WontDo)` in tree, `goals::Model` delegates);
`find_open_by_coaching_session_id` in `entity_api/src/action.rs` (Rust-side `!status.is_completed()`
filter, returns lean `Vec<actions::Model>`). Adding a non-`Option` NOT NULL column forced mechanical
`ical_sequence: 0`/`Unchanged(...)` edits across 17 more files (all Model literals, ActiveModel
initializers, and frozen expected-SQL strings); overseer verified every edit is mechanical and that
NO update path uses `Set` (would reset the sequence). Tests: `entity` status 3, `entity_api` action 50
(mock), full workspace compiles; clippy/fmt clean. Migration apply/revert: inspection-only (no local DB
wired), CI exercises it.
Note (informs Phase 4): `coaching_sessions.title` and a `coaching_session_topics` entity now exist
post-merge, so the Phase 1 composer can be fed real title/topics.
- Migration (after #356's series migration): add `ical_sequence INTEGER NOT NULL DEFAULT 0` to **both** `coaching_sessions` **and** `coaching_session_series` (singles use the session column for UID `<session_id>`; series use the series column for UID `<series_id>`). Update both entities (`ical_sequence: i32`).
- `entity_api/src/action.rs`: `find_by_coaching_session_id(db, session_id)` returning open actions. "Open" reuses the existing shared status definition: `goals::Model::is_completed()` (`entity/src/goals.rs:87`) defines completed as `Completed | WontDo`. Promote that to a `Status` method (e.g. `Status::is_completed()` in `entity/src/status.rs`) and have both goals and actions use it; the query filters `!status.is_completed()`. Do **not** hand-roll a new `status != Completed` predicate.
- **Acceptance:**
  - [x] Migration adds `ical_sequence` to both tables (compiles + registered; DB apply/revert deferred to CI); entities expose `ical_sequence: i32`.
  - [x] `Status::is_completed()` exists and `goals::Model::is_completed()` delegates to it (no duplicated definition).
  - [x] `find_open_by_coaching_session_id` returns `NotStarted`/`InProgress`/`OnHold` actions and excludes `Completed`/`WontDo` (mock-DB test).

### Phase 4 — Attach `.ics` to existing create emails (scenarios 1 & 2)
- `notify_session_scheduled` (scenario 1): load topics/goals/open-actions, build `IcsInvite` (single, `REQUEST`, UID `<session_id>`, seq 0, anchor = coach TZ), attach via `add_ics_attachment` in `send_session_email_to_recipient`. Build the `.ics` **once** per session (same VEVENT for both recipients).
- `notify_recurring_sessions_scheduled` (scenario 2): build `IcsInvite` with `recurrence = Some(series.rule.recurrence)`, UID `<series_id>`, seq 0. This requires the function to also receive the `coaching_session_series` model (id + rule); it currently takes only `&[sessions]`, so change the signature and update the #356 series-create controller call site to pass the series.
- **Acceptance:**
  - [ ] New single-session emails to both parties carry `invite.ics`; opening it creates the event with correct time, organizer/attendee, working Join button from `meeting_url`, and a clickable in-app link.
  - [ ] Coach `America/New_York` 3 PM ET → coachee `America/Los_Angeles` sees 12 PM PT; switching device to `America/Chicago` re-renders at 2 PM CT (anchored to NY).
  - [ ] Outlook desktop renders the single-session event at the coach's correct local time (TZID-drop regression guard).
  - [ ] Series create sends one email per recipient; the single `.ics` `RRULE` materializes the full series with each occurrence handling DST in the anchor TZ, UID == `<series_id>`.
  - [ ] Single-session description shows title/topics/goals/actions per the composition rules; series description shows link + first-session goals only.
  - [ ] Existing `mockito` body-match tests updated to expect `attachments` + ≥1 structural `.ics` assertion (UID prefix, METHOD, SEQUENCE, `VTIMEZONE` present, TZID == coach zone). Series test asserts `RRULE` + `<series_id>` UID.

### Phase 5 — Net-new update/reschedule emails (scenarios 3 & 4)
- **Calendar-relevant change set (decided):** re-send when **any** field that appears in the invite changes — `date`, `duration_minutes`, `meeting_url`, or description fields (`title`, topics, goals, open actions). A change to any of these produces an updated `.ics` so the calendar entry stays accurate; an edit touching none of them sends nothing.
- **Single (scenario 3):** thread `&Config` into `coaching_session::update` + its two call sites; detect a calendar-relevant change (compare pre/post), bump `coaching_sessions.ical_sequence`, fire new `notify_session_rescheduled` (`REQUEST`, same UID, bumped SEQUENCE) with a new `SessionRescheduled` impl + template.
- **Series (scenario 4):** hook `series::reschedule` (or its controller): bump `coaching_session_series.ical_sequence`, fire new `notify_series_rescheduled` (`REQUEST` + `RRULE`, UID `<series_id>`, bumped SEQUENCE).
- **Acceptance:**
  - [ ] A `PUT` changing `date`, `duration`, `meeting_url`, `title`, topics, goals, or open actions sends a reschedule email; opening the `.ics` updates the event in place (same UID, bumped SEQUENCE) on all three platforms.
  - [ ] A `PUT` changing none of those fields sends no reschedule email.
  - [ ] Rescheduling a series (`PUT /coaching_session_series/:id`) sends an email; opening the `.ics` updates the recurring event in place (same `<series_id>` UID, bumped SEQUENCE).

### Phase 6 — Net-new cancel emails (scenarios 5 & 6)
- **Single (scenario 5):** in `coaching_session::delete`, bump `ical_sequence` and fire new `notify_session_cancelled` (`CANCEL`, `STATUS:CANCELLED`, bumped SEQUENCE) **before** the DB delete (record must still exist). New `SessionCancelled` impl + template.
- **Series (scenario 6):** in `series::delete_with_future_sessions`, bump series `ical_sequence` and fire new `notify_series_cancelled` (`CANCEL` + `RRULE`, UID `<series_id>`) before delete. Note #356 keeps past sessions as orphans; the series `CANCEL` removes the recurring event from calendars (past occurrences already happened — acceptable).
- **Acceptance:**
  - [ ] Deleting a single session sends a cancellation; opening the `.ics` removes the event from all three platforms.
  - [ ] Deleting a series (`DELETE /coaching_session_series/:id`) sends a cancellation; opening the `.ics` removes the recurring event.
  - [ ] `STATUS:CANCELLED` + bumped SEQUENCE asserted; the email fires before the DB delete.

### Phase 7 — Config / env passthrough + manual setup
- New template-id flags for the net-new emails (Phases 5/6). **Templates shared as much as logical:** single and series share a `rescheduled` template and a `cancelled` template, differentiated by a singular/series wording variable (rather than dedicated templates each). Add only the flags this implies.
- Wire every new flag through **all** layers per `.claude/CLAUDE.md`: `service/src/config.rs` (flag list, struct fields, `debug_field`, getters); `docker-compose.yaml`; `docker-compose.pr-preview.yaml`; `.github/workflows/deploy_to_do.yml` (.env heredoc); `.github/workflows/ci-deploy-pr-preview.yml` (heredoc + `secrets:`/inputs). Secret values → `production` + `PR_PREVIEW_*`.
- Create the Resend templates (subjects on the template; never in payload).
- **Acceptance:**
  - [ ] `cargo run -- --help` shows every new flag; each new var name appears in both compose files and both deploy workflows.
  - [ ] Reschedule and cancel emails render correctly in a PR preview env with the shared templates.

### Phase 8 — Per-occurrence cancellation within a series (scenario 7)
Cancel a single coaching session that belongs to a series, sending an `.ics` update that removes only
that one occurrence from attendees' calendars (the rest of the series stays).
- Implementation: when `coaching_session::delete` (or a dedicated "remove occurrence" path) targets a
  session whose `coaching_session_series_id` is set, the series recurring event must gain an `EXDATE`
  for that occurrence's `DTSTART`, OR emit a `CANCEL` with a `RECURRENCE-ID` identifying the instance.
  Decide which during the phase (EXDATE on a re-issued series `REQUEST` is the more broadly compatible
  option; `RECURRENCE-ID` + `CANCEL` is more surgical but has spottier client support). Bump the series
  `ical_sequence`.
- **Acceptance:**
  - [ ] Deleting one session within a series sends an `.ics` that removes exactly that occurrence on Google + Apple + Outlook, leaving the remaining occurrences intact.
  - [ ] The series UID is unchanged; SEQUENCE is bumped.

---

## Out of scope (v1)
- Per-user opt-out of calendar attachments.
- Per-session/series immutable timezone (anchor captured at create time). v1 uses the coach's current TZ at send time.
- Inbound RSVP processing.
- Surface changes outside email (in-app calendar view).

## Resolved decisions (from review)
- **Calendar-relevant change set:** `date`, `duration_minutes`, `meeting_url`, and description fields (`title`, topics, goals, open actions) all trigger a reschedule email (Phase 5).
- **Template reuse:** single + series share `rescheduled`/`cancelled` templates with a wording variable (Phase 7).
- **Open-action definition:** reuse the existing `is_completed()` semantics (`Completed | WontDo`); open = `!is_completed()`. Promote to a shared `Status` method (Phase 3).
- **Missing `users.timezone` in `VTIMEZONES`:** fall back to UTC `Z` encoding (no `VTIMEZONE`), log a warning (Phase 1).
- **Series description:** show the first session's in-progress goals (ongoing cross-session objectives) + in-app link; omit title/topics/actions (per-session).
- **Per-occurrence series cancellation:** in scope as the final phase (Phase 8), not deferred.
- **`notify_recurring_sessions_scheduled` signature:** will be changed to receive the `coaching_session_series` model (id + rule); coordinate the call-site change with #356 (Phase 4).
