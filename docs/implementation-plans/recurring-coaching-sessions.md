# Recurring Coaching Sessions — Batch Create with Lazy Hydration

## Context

Coaches today can only schedule one session at a time via `POST /coaching_sessions`. They want to schedule a recurring series (e.g. "every Mon + Wed for 12 weeks") in a single request, without us introducing a new "series" entity.

The catch: the current `create` flow eagerly creates a Tiptap collab document, mints a meeting URL via OAuth (Google Meet / Zoom API), and links in-progress goals to the session. Doing all of that for, say, 24 sessions in one request fans out to ~50 external API calls — slow, error-prone, and creates lots of resources nobody may ever use (a coachee may cancel the series after week 2).

**Outcome we want:** a new `POST /coaching_sessions/recurring` endpoint that bulk-inserts up to ~1 year of session rows in one transaction with **none** of the heavy side-effects performed at create time. Tiptap doc, meeting URL, and goal-linking are all deferred to **first load** by either the coach or the coachee — meaning the first time `GET /coaching_sessions/:id` is hit for that session.

## Non-goals (explicitly out of scope for this task)

- No `coaching_session_series` parent table. Each session row stands alone; the only thing tying them together is that they were created in the same request.
- No "edit/cancel the whole series" endpoints. Per-session edit/delete works as today.
- No recurrence-rule storage on the session row. We expand the rule into rows once at create time and discard it.
- No change to the existing `POST /coaching_sessions` single-create path or its eager side-effects. That endpoint stays exactly as-is.

## Design Overview

### Sentinel for "not yet hydrated"

We add one new nullable column to `coaching_sessions`:

- `hydrated_at TIMESTAMPTZ NULL`

**Invariant:** `hydrated_at IS NOT NULL` ⇒ hydration ran to a definitive decision for this row. Specifically:
- Tiptap collab document was created.
- In-progress goals from the coaching_relationship were linked.
- Provider + meeting URL were either both resolved (coach had an OAuth connection at hydration time) or both deliberately left `NULL` (no OAuth connection found).

Hydration is a one-shot operation. Once `hydrated_at` is set, we never re-attempt — matching the existing single-create behavior, where a session created without an OAuth connection permanently has no meeting URL.

**Who populates it:**
- Existing `POST /coaching_sessions` (eager single-create) — sets `hydrated_at = NOW()` on insert. All three steps run inside the create transaction before the row is committed, so the invariant holds at commit time.
- New `POST /coaching_sessions/recurring` (batch create) — inserts rows with `hydrated_at = NULL`. Each row gets populated the first time it's loaded via `GET /coaching_sessions/:id`.
- Migration backfill — pre-existing rows get `hydrated_at = created_at`, since the old eager path already ran for them.

In steady state, every row in the table has `hydrated_at` set **except** rows created by the recurring endpoint that nobody has opened yet.

**Why a dedicated column rather than reusing `collab_document_name IS NULL`?** Two reasons. (1) The conditional UPDATE that claims the hydration race needs an unambiguous "claim" target. (2) If a single-create row ever legitimately had `collab_document_name = NULL` (e.g. Tiptap was down at create-time and we chose to soldier on), we wouldn't want it accidentally re-hydrating later. `hydrated_at` cleanly separates "the side-effects ran" from "the resulting column happens to be non-null."

### New endpoint

`POST /coaching_sessions/recurring` — request body:

```json
{
  "coaching_relationship_id": "...",
  "start_at": "2026-05-15T10:00:00",
  "recurrence": {
    "frequency": "weekly" | "biweekly" | "monthly" | "daily",
    "interval": 1,
    "by_weekdays": ["Mon", "Wed"],
    "count": 24,
    "until": null
  }
}
```

- `start_at` is the first occurrence (analogous to the existing `date` field).
- **No `provider` field.** Unlike single-create, the recurring endpoint does not accept a provider — it's resolved at hydration time from the coach's OAuth connection (see "Provider resolution at hydration" below).
- `recurrence.interval` defaults to 1.
- `recurrence.by_weekdays` is **only** accepted when `frequency == weekly` (and biweekly, treated as `weekly` with `interval = 2`). Rejected otherwise.
- Exactly one of `count` or `until` must be provided.
- For `monthly`, the day-of-month comes from `start_at`. If a target month doesn't have that day (e.g. day 31 in February), we clamp to the last day of that month.
- Response: `201 Created` with `{ "sessions": [Model, ...] }` — every row has `hydrated_at = null`, `provider = null`, `collab_document_name = null`, `meeting_url = null`, no goal links.

### Validation & 1-year cap

A single `validate_recurrence` helper enforces:

1. Total occurrence count ≤ `MAX_RECURRING_OCCURRENCES = 365` (covers daily-for-a-year worst case).
2. Span from `start_at` to the last generated occurrence ≤ `366 days` (covers leap years).
3. `interval >= 1`.
4. `by_weekdays` non-empty if provided; only valid with weekly/biweekly.
5. `start_at` not in the past (consistent with single-create).
6. If `by_weekdays` is set, `start_at`'s weekday must be in `by_weekdays` (avoids ambiguity about whether to skip the first week).

Both caps are enforced — the user said "1 year regardless of frequency". Whichever bound is tighter wins.

### Recurrence expansion

A pure function `expand_recurrence(start_at, rule) -> Result<Vec<NaiveDateTime>, Error>` generates the dates. Lives in `domain/src/coaching_session/recurrence.rs` (new file). Uses `chrono` (already in tree at [domain/src/coaching_session.rs:6](../../domain/src/coaching_session.rs#L6)). No external RRULE crate — the rule space is small enough.

Algorithm sketch:
- `daily`: step `interval` days N times.
- `weekly` with no `by_weekdays`: step `interval * 7` days N times.
- `weekly` with `by_weekdays`: walk week-by-week (`interval` weeks at a time); within each week, emit each weekday in `by_weekdays` that's ≥ `start_at`.
- `biweekly`: same as `weekly` with `interval = 2`.
- `monthly`: step `interval` months, clamping day-of-month to month length.

### Domain-layer bulk_create

New function: `domain::coaching_session::bulk_create_recurring(db, relationship_id, dates) -> Result<Vec<Model>, Error>`.

- Fetches the coaching relationship once (validates it exists; AuthZ on top of this).
- Builds N `coaching_sessions::ActiveModel`s with `provider = NULL`, `collab_document_name = NULL`, `meeting_url = NULL`, `hydrated_at = NULL`. No provider is taken from the request.
- Single transaction, single `Entity::insert_many(...).exec_with_returning(&txn)` (SeaORM supports this for Postgres). One round-trip, atomic.
- **Does not** call Tiptap, OAuth, meeting providers, or goal-link helpers. All deferred.
- Returns the inserted models.

### Read-path lazy hydration

The hydration trigger lives in the existing `read` handler at [web/src/controller/coaching_session_controller.rs:39](../../web/src/controller/coaching_session_controller.rs#L39). It already routes through the `CoachingSessionAccess` extractor which enforces coach-or-coachee membership — so by the time we hydrate, AuthZ has already passed.

New domain function: `domain::coaching_session::ensure_hydrated(db, config, session_id) -> Result<Model, Error>`.

```
ensure_hydrated(session_id):
  1. Acquire pg_advisory_xact_lock(session_id_hash) inside a short transaction.
  2. Re-read the session row. If hydrated_at IS NOT NULL → commit and return as-is.
  3. Look up the coaching_relationship + organization (needed for Tiptap doc name and OAuth).
  4. Run the deferred side-effects, adapted for idempotency:
     a. Tiptap.create(doc_name) — already idempotent (returns 200 or 409 — see domain/src/gateway/tiptap.rs).
     b. Resolve provider + meeting URL (see "Provider resolution at hydration" below).
     c. link_in_progress_goals_to_session(&txn, ...) — see "Goal-link idempotency fix" below.
  5. UPDATE coaching_sessions
       SET collab_document_name = ?,
           provider = ?,            -- resolved provider or NULL
           meeting_url = ?,         -- minted URL or NULL
           hydrated_at = NOW()
       WHERE id = ?.
  6. Commit. Return refreshed model.
  On error after step 4a: compensate by deleting the Tiptap doc (matches existing pattern at domain/src/coaching_session.rs:101-105).
```

Then in the `read` handler:

```rust
let session = if coaching_session.hydrated_at.is_none() {
    CoachingSessionApi::ensure_hydrated(db, &config, coaching_session.id).await?
} else {
    coaching_session
};
```

This is the only read path that triggers hydration. `index` (list) and `find_by_user_with_includes` deliberately do **not** trigger — listing 50 sessions must not fan out into 50 hydrations.

**Defensive guard:** also call `ensure_hydrated` in `jwt_controller::generate_collab_token` (the endpoint that mints the JWT for connecting to the Tiptap collab editor). This protects against a frontend that somehow opens the editor without first hitting GET-by-id. Idempotent thanks to step 2.

### Provider resolution at hydration

The recurring endpoint never accepts a provider — rows are inserted with `provider = NULL`. Hydration resolves it as follows:

1. Look up the coach's OAuth connection via a new helper `domain::oauth_connection::find_by_user(user_id)`. The OAuth flow enforces at most one connection per coach, so this returns `Option<oauth_connections::Model>` rather than a list.
2. **If found:** set `provider` to that connection's provider, then mint the meeting URL via the existing `create_meeting_url` helper at [domain/src/coaching_session.rs:223](../../domain/src/coaching_session.rs#L223). For persistent-URL providers (Google Meet), reuse an existing URL from the relationship via [`find_reusable_meeting_url`](../../domain/src/coaching_session.rs#L198) before minting.
3. **If not found:** leave both `provider` and `meeting_url` as `NULL`. The session is still usable — coach and coachee can join via whatever channel they communicate through. This mirrors the existing single-create behavior at [domain/src/coaching_session.rs:180-191](../../domain/src/coaching_session.rs#L180-L191) where missing credentials produce a session without a meeting URL.

The resolved provider is **written back** to `coaching_sessions.provider` so subsequent reads are stable — the row tells a coherent story ("this session was Zoom") regardless of whether the coach's OAuth state changes later.

### Concurrency: Postgres advisory lock

User chose advisory lock. Implementation:

- Use `pg_try_advisory_xact_lock(hash_session_id(session_id))` with `hash_session_id` mapping the UUID to an `i64` (e.g. take the first 8 bytes of the UUID big-endian). Transaction-scoped — auto-released on commit/rollback.
- If `try_advisory_xact_lock` returns `false` (another request is hydrating right now), block on `pg_advisory_xact_lock` (the non-`try` variant) so the second caller sleeps, then re-reads the row, sees `hydrated_at IS NOT NULL`, and returns.
- We accept that this holds the row's hydration lock for the duration of external HTTP calls to Tiptap + Zoom (potentially seconds). Since the lock is keyed per-session and only blocks other hydration attempts on the same session, the blast radius is small.

Belt-and-suspenders idempotency, even with the lock:
- **Tiptap** — `.create()` already 200/409-idempotent ([domain/src/gateway/tiptap.rs](../../domain/src/gateway/tiptap.rs)).
- **Goal linking** — see fix below.
- **Meeting URL** — guarded by the `hydrated_at IS NOT NULL` short-circuit at step 2 of `ensure_hydrated` (a concurrent winner sets `hydrated_at` and commits before releasing the lock; the loser re-reads inside its own lock acquisition and sees the populated row). Additionally, `find_reusable_meeting_url` returns the relationship's existing URL for persistent providers, so we never mint a duplicate Google Meet space.

### Goal-link idempotency fix

`link_in_progress_goals_to_session` at [entity_api/src/coaching_session_goal.rs:299](../../entity_api/src/coaching_session_goal.rs#L299) calls `insert_link_row` in a loop, which fails on the unique constraint `(coaching_session_id, goal_id)` on retry. Fix: change `insert_link_row` to use `ON CONFLICT (coaching_session_id, goal_id) DO NOTHING` via SeaORM's `OnConflict` builder. The function still returns the count of newly linked goals (rows-affected); concurrent re-entry just sees zero new links.

This change is safe for the existing eager-create caller too — that path runs inside its own transaction and never re-runs against the same session, so the no-op behavior never fires there.

### Email behavior

The existing `coaching_session_controller::create` calls `EmailsApi::notify_session_scheduled` per created session. Sending one email per session for a 24-week series is spam.

For the recurring endpoint: send **one** email summarizing the series (count, frequency, first occurrence, last occurrence). New helper `EmailsApi::notify_recurring_sessions_scheduled(db, config, relationship, &sessions)` in [domain/src/emails.rs](../../domain/src/emails.rs). The body lists all dates; subject is "N recurring sessions scheduled".

## Files to Create / Modify

### Create

- `migration/src/m20260510_000000_add_hydrated_at_to_coaching_sessions.rs` — adds `hydrated_at TIMESTAMPTZ NULL` to `coaching_sessions`. Backfills existing rows with `created_at` (treat all pre-existing rows as already hydrated). Registered in [migration/src/lib.rs](../../migration/src/lib.rs).
- `domain/src/coaching_session/recurrence.rs` — `Recurrence` struct, `Frequency` enum, `Weekday` enum (or reuse `chrono::Weekday`), `expand_recurrence`, `validate_recurrence`. Pure functions, unit-tested.
- `web/src/params/coaching_session/recurring.rs` — request DTO (`CreateRecurringParams`) with `serde` + `validator` derive.

### Modify

- `entity/src/coaching_sessions.rs` — add `hydrated_at: Option<DateTimeWithTimeZone>`.
- `domain/src/coaching_sessions.rs` (re-export wrapper) — same field if applicable.
- `domain/src/coaching_session.rs`:
  - Add `bulk_create_recurring(db, params) -> Result<Vec<Model>, Error>`.
  - Add `ensure_hydrated(db, config, session_id) -> Result<Model, Error>`.
  - Existing `create` sets `hydrated_at = Some(now)` on the model before insert (so the read-path hydration check skips single-create rows, which have already had all three steps run inside the create transaction).
  - Refactor `maybe_attach_meeting_url` into reusable helpers. Specifically: split into `resolve_provider_via_oauth(db, coach_id) -> Option<Provider>` and `mint_meeting_url(db, config, coach_id, provider, start_time, external_account_id) -> Result<String, Error>` so `ensure_hydrated` can use the "any provider" resolution path while the existing `create` flow keeps using its explicit-provider path.
- `entity_api/src/coaching_session_goal.rs::insert_link_row` — switch to `ON CONFLICT DO NOTHING` insert. Update docstring on `link_in_progress_goals_to_session` to mention idempotency.
- `web/src/controller/coaching_session_controller.rs`:
  - Add `create_recurring` handler. AuthZ: extract authenticated user, fetch the coaching_relationship, check `user.id == coach_id` (only coaches schedule sessions; coachees can't). This matches the spirit of existing protect middleware at [web/src/protect/coaching_sessions.rs:21-46](../../web/src/protect/coaching_sessions.rs#L21-L46).
  - Modify `read` to call `ensure_hydrated` when `hydrated_at.is_none()`.
- `web/src/controller/jwt_controller.rs::generate_collab_token` — defensive `ensure_hydrated` call before issuing the token.
- `web/src/router.rs`:
  - Register `POST /coaching_sessions/recurring` route in [coaching_sessions_routes()](../../web/src/router.rs#L200) under `require_auth`.
  - Register `coaching_session_controller::create_recurring` in the OpenAPI `paths(...)` block.
- `web/src/protect/coaching_sessions.rs` — add a `create_recurring` middleware mirroring the relationship-coach check.
- `domain/src/emails.rs` — add `notify_recurring_sessions_scheduled`.

## Reused existing utilities

- `domain/src/gateway/tiptap.rs` — `TiptapDocument::new` + `.create()` (already idempotent on 409).
- `domain/src/coaching_session.rs::find_reusable_meeting_url` — meeting URL reuse for persistent providers.
- `domain/src/coaching_session.rs::create_meeting_url` — Google Meet / Zoom minting via OAuth.
- `domain::oauth_connection::find_by_user_and_provider` + `get_valid_access_token` (still used by the eager single-create path).
- New helper `domain::oauth_connection::find_by_user(user_id) -> Option<oauth_connections::Model>` — used by hydration's provider resolution. Returns the coach's single OAuth connection if one exists. Safe to expose because the OAuth flow enforces the one-connection-per-user invariant.
- `entity_api/src/coaching_session_goal.rs::link_in_progress_goals_to_session` (with idempotency fix).
- `entity_api/src/coaching_session.rs::find_meeting_url_by_relationship_and_provider`.
- `web/src/extractors/coaching_session_access.rs` — coach-or-coachee gate already used by `read`.
- `chrono` — already in tree, sufficient for date math.
- `MAX_IN_PROGRESS_GOALS` constant pattern at [entity_api/src/goal.rs:15](../../entity_api/src/goal.rs#L15) — model the new `MAX_RECURRING_OCCURRENCES` constant the same way.

## Verification

1. **Unit tests** (no DB, pure logic):
   - `expand_recurrence`: weekly (interval 1, 2, 3), weekly with by_weekdays (Mon+Wed, Tue+Thu+Fri), biweekly, monthly with day-of-month clamping (Jan 31 → Feb 28), daily.
   - `validate_recurrence`: rejects > 365 occurrences, > 366-day span, both `count` and `until` set, `by_weekdays` on non-weekly, weekday mismatch with `start_at`, past `start_at`.

2. **Integration tests** with `MockDatabase` (existing pattern in [domain/src/coaching_session.rs:283-484](../../domain/src/coaching_session.rs#L283-L484)):
   - `bulk_create_recurring` inserts N rows in one transaction with `provider = None`, `meeting_url = None`, `collab_document_name = None`, `hydrated_at = None`. No Tiptap call, no OAuth call, no goal-link call.
   - `ensure_hydrated` with OAuth connection: re-reads, creates Tiptap doc, resolves provider from the connection, mints meeting URL, links goals, sets `provider` + `meeting_url` + `hydrated_at` in the final UPDATE. Verify second call no-ops on hydrated row.
   - `ensure_hydrated` without OAuth connection: re-reads, creates Tiptap doc, leaves `provider` and `meeting_url` as `NULL`, still links goals and sets `hydrated_at`. Verify second call no-ops (no second OAuth lookup, no retry).
   - `ensure_hydrated` on an already-hydrated row: only one SELECT, no side-effect mocks fire.

3. **Manual end-to-end** against a running stack (`cargo run` + frontend on PR preview):
   - As coach with an active Zoom OAuth connection: `POST /coaching_sessions/recurring` with weekly+Mon+Wed, count=4 → returns 8 sessions, all with `provider=null`, `meeting_url=null`, `collab_document_name=null`, `hydrated_at=null`. DB inspection confirms.
   - As same coach: `GET /coaching_sessions/:id` for one of them → response now has `provider="zoom"`, populated `meeting_url`, `collab_document_name`, and non-null `hydrated_at`. Tiptap dashboard shows the doc was created. Zoom dashboard shows the meeting was created.
   - Second `GET /coaching_sessions/:id` → identical response, no new external calls (verify via logs / external-system dashboards).
   - As coachee: `GET /coaching_sessions/:id` for a sibling session → triggers hydration the same way; coach-side `GET` afterwards shows hydrated row.
   - **No-OAuth path:** as a coach with no OAuth connection, `POST /coaching_sessions/recurring` → returns sessions as above. `GET /coaching_sessions/:id` for one of them → `provider` and `meeting_url` remain `null`, but `hydrated_at` is set, Tiptap doc is created, and goals are linked. A subsequent `GET` does not re-attempt OAuth resolution.
   - Goal auto-link: create an in-progress goal on the relationship, then `GET` an unhydrated session → goal is now linked (verify via `GET /coaching_sessions/:id/goals`).

4. **Concurrency check** (manual, optional): use two `curl` processes hitting `GET /coaching_sessions/:id` for the same unhydrated session in parallel. Logs should show one hydration path running, the other waiting on the advisory lock and then returning the now-hydrated row. Tiptap and Zoom dashboards should each show exactly one resource created.

5. **Lint & format gates** before commit: `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --check` (per project conventions in [.claude/CLAUDE.md](../../.claude/CLAUDE.md)).

## Open questions to confirm during implementation

- Should `start_at` enforce the minute-truncation that single-create does (`SessionDate::new` at [domain/src/coaching_session.rs:32](../../domain/src/coaching_session.rs#L32))? Default: yes, apply the same truncation to every generated occurrence.
- Time zones: `start_at` is `NaiveDateTime` today. Recurrence expansion in naive time may drift across DST. For MVP, we add the same naive offsets and accept a 1-hour drift twice a year — call this out in the API doc and revisit with timezone support later.
