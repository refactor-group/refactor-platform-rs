# Coaching Session Duration & Per-Coach Default Preference

## Context

Today `coaching_sessions` carries `date` (start time) but no duration. Every consumer that needs an end time has to assume one. The immediate driver is GitHub issue [#333](https://github.com/refactor-group/refactor-platform-rs/issues/333) (`.ics` calendar invites), which cannot compute `DTEND` without a duration field. The duration is also surfaced in the existing session-scheduled and recurring-sessions-scheduled emails so recipients see how long their session runs.

This implements [#332](https://github.com/refactor-group/refactor-platform-rs/issues/332): two new fields land — `coaching_sessions.duration_minutes` and `users.default_coaching_session_duration_minutes` (the per-coach default) — plus a defaulting cascade so that omitting `duration_minutes` from a session-create request resolves to the coach's stored preference.

## Branch

Create a new branch off `main` before starting work. Suggested name: `feat/coaching-session-duration` (matches the existing prefix convention, e.g. `feat/dashboard-session-buckets`, `feat/users-coaching-sessions-tz-param`).

## Coding-standards update (first commit on the branch)

Add a new subsection to [.claude/coding-standards.md](.claude/coding-standards.md) under "Module Organization", positioned between "Layer Responsibilities" and "Thin Controllers". This documents the entity-type re-export boundary pattern that the layered access in this plan depends on, and that wasn't previously captured in the standards. Land this as the first commit on the branch so the rest of the feature work can follow a documented pattern.

The exact text to add:

````markdown
### Type Re-Export Boundary

Entity types — whether table Models (e.g., `coaching_sessions::Model`) or non-table value types that constrain column values (e.g., `Provider`, `Id`, `Duration`) — are declared once in the `entity` crate and re-exported up through each layer at the `lib.rs` level. The web layer accesses them via `domain::<module>::<Type>` and **must never reach across into `entity_api` or `entity` directly**.

**The re-export chain:**

```rust
// entity/src/lib.rs — declare the module
pub mod provider;
pub mod duration;
// ...
pub type Id = Uuid;

// entity_api/src/lib.rs — re-export entity modules in the existing block
pub use entity::{
    actions, agreements, coaching_sessions, /* ... */
    provider, duration, users::Role, Id,
};

// domain/src/lib.rs — re-export entity_api re-exports in the existing block
pub use entity_api::{
    actions, agreements, coaching_sessions, /* ... */
    provider, duration, users::Role, Id,
};
```

**Access paths by layer:**

| Layer | Import for a type like `Duration` |
|---|---|
| Inside `entity_api/*.rs` | `use crate::duration::Duration;` |
| Inside `domain/*.rs` | `use crate::duration::Duration;` |
| Inside `web/*.rs` | `use domain::duration::Duration;` |

**Why this pattern:** As documented in `domain/src/lib.rs`, "consumers of the `domain` crate do not need to directly depend on the `entity_api` crate." The same isolation applies one layer down: entity_api hides direct dependence on entity. This means a layer can be refactored internally without breaking layers above it, and the type-import surface area at the web layer stays uniform regardless of where types originate.

**Common mistake:** Adding a `pub use entity::<module>::<Type>;` re-export inside a domain module file (e.g., `domain/src/coaching_session.rs`). This reaches across the entity_api boundary and bypasses the documented isolation. Re-exports of entity types belong at `lib.rs` only — never inside individual modules.

**When adding a new entity type:**

1. Declare the module in `entity/src/lib.rs`: `pub mod <name>;`.
2. Add it to the existing `pub use entity::{...};` block in `entity_api/src/lib.rs`.
3. Add it to the existing `pub use entity_api::{...};` block in `domain/src/lib.rs`.
4. Internal code in each crate imports via `crate::<name>::<Type>`. Web code imports via `domain::<name>::<Type>`.

The error-type chain (`entity_api::error` → `domain::error` → `web::Error`) is a parallel pattern with its own conversion rules — see "Cross-Layer Error Propagation" above. Both patterns share the same architectural principle: each layer hides the layers below it from the layers above.
````

Also append a corresponding bullet to the "Code Review Checklist" at the bottom of the file:

```markdown
- [ ] Entity-derived types are accessed via `domain::<module>::<Type>` in web code (never `entity_api::...` or `entity::...`)
```

## Related Issues

| Issue | Repo | Status | Relevance |
|---|---|---|---|
| [#332](https://github.com/refactor-group/refactor-platform-rs/issues/332) | refactor-platform-rs | open | This plan |
| [#333](https://github.com/refactor-group/refactor-platform-rs/issues/333) | refactor-platform-rs | blocked-on-#332 | Depends on `duration_minutes` |
| TBD | refactor-platform-fe | coordinated via collab board (`frontend_duration_ui_issue_request`) | Schedule/reschedule dialog + user-settings UI |

## Architecture

### Schema placement

The `default_coaching_session_duration_minutes` field lives on the `users` table directly, mirroring the existing `users.timezone` pattern (per-user settings where every user has a value, even when its meaning is role-dependent). Cost: 2 bytes per non-coach user (negligible). Benefit: single-table reads, no JOINs, no "row not found" edge case, matches established convention.

If 3+ coach-specific preferences land in the future, extracting a `coach_settings` table is a clean future refactor with well-defined scope. Not in scope for v1.

### Defaulting cascade

When `duration_minutes` is omitted from a session-create payload:
1. Look up the coach via the relationship's `coach_id`.
2. Use `coach.default_coaching_session_duration_minutes` (validated by DB invariant — `NOT NULL DEFAULT 60`).

When present, the requested value is validated and used as-is.

Cascade resolution lives in `entity_api`, not domain. Project precedent: commit `45e9431` ("Move active goal limit validation from domain to entity_api") established that entity-level invariants and resolution belong at the entity_api layer. The cascade also needs a DB lookup (coach by id), which is naturally entity_api territory.

### `Duration` newtype

A validated `Duration(u16)` newtype lives in **`entity/src/duration.rs`** — a new top-level module in the entity crate. It owns four concerns: the unsigned domain (no negative durations), the validation rule (`1..=480`), the storage type bound (`u16` → PG `SMALLINT`), and the human-readable formatting (via `Display`).

**Why entity and not entity_api?** Entity is where types representing the *shape* of data live. It already contains non-table types that constrain the value set of specific columns: [`entity/src/provider.rs`](entity/src/provider.rs) (the `Provider` enum used by `coaching_sessions.provider`) and `Id` (the UUID newtype in `entity/src/lib.rs` used as primary keys). `Duration` is structurally identical to `Provider`: a constrained type representing the value set of a DB column. It belongs alongside those types in entity. Entity_api stays focused on *operations* (CRUD, cascades, cross-entity lookups) and consumes `Duration` from entity.

**Why a newtype and not a `format_duration` free function + standalone validator?** The newtype absorbs four otherwise-separate constructs (validation function, format function, `MIN/MAX` consts, type bound) into one with clean invariants. One construct, one source of truth, idiomatic Rust.

**Why keep the SeaORM Model field as `u16` (not `Duration`)?** SeaORM's native type bindings work cleanly on primitives; teaching it to serialize a newtype via custom `TryGetable`/`Value` impls is fightable but unnecessary friction. The smart-constructor pattern keeps storage primitive and converts at the API boundary.

### Validation boundary

`Option<u16>` exists only at the wire layer (JSON deserializes to primitives). The controller converts `Option<u16>` → `Option<Duration>` via `try_from` immediately on receipt — invalid values return 422 right there. Every function signature in domain and entity_api takes `Option<Duration>` from that point on, so the type system carries the "validated" property without runtime cost.

The one exception is the update path, which flows through the existing `IntoUpdateMap → HashMap<String, sea_orm::Value>` pattern (values are erased to primitives by the generic patch abstraction). For that path, entity_api re-validates at extraction via `Duration::try_from` before writing — the small cost of one extra validation on a field that's already been validated upstream is preferable to forcing every patchable field to grow its own newtype just to preserve type safety across the map boundary.

```rust
pub const MIN_DURATION_MINUTES: u16 = 1;
pub const MAX_DURATION_MINUTES: u16 = 480;
pub const DEFAULT_DURATION_MINUTES: u16 = 60;

/// A validated coaching-session duration in minutes (1..=480).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duration(u16);

impl Duration {
    pub fn new(minutes: u16) -> Result<Self, Error> { /* range check */ }
    pub const fn from_minutes_unchecked(minutes: u16) -> Self { Self(minutes) }
    pub const fn minutes(self) -> u16 { self.0 }
}

impl TryFrom<u16> for Duration {
    type Error = Error;
    fn try_from(v: u16) -> Result<Self, Self::Error> { Self::new(v) }
}

impl std::fmt::Display for Duration {
    /* All eight singular/plural cases:
       1 → "1 minute"     45 → "45 minutes"
       60 → "1 hour"      120 → "2 hours"
       61 → "1 hour 1 minute"        62 → "1 hour 2 minutes"
       121 → "2 hours 1 minute"      122 → "2 hours 2 minutes"  */
}
```

Use sites:
- **Validate input:** `Duration::try_from(params.duration_minutes)?`
- **Format for email:** `Duration::from_minutes_unchecked(session.duration_minutes).to_string()`

`from_minutes_unchecked` is a deliberate escape hatch for already-trusted DB values, so we don't re-validate on every read. DB invariants (`NOT NULL DEFAULT 60` + the only writes go through `Duration::new`) keep stored values inside the range.

### Wire-shape decoupling

`POST /coaching_sessions` currently takes `Json<Model>` directly. To distinguish "client omitted the field" from "client sent a value," the controller's request body switches to a new `CreateParams` struct with `duration_minutes: Option<u16>`. The entity `Model` keeps `duration_minutes: u16` (mirroring the `NOT NULL` column).

This decoupling is overdue regardless of duration — it brings the create endpoint in line with the existing `UpdateParams` pattern and gives future per-session fields a home that doesn't pollute the entity.

## Files Modified

### Migrations

Two new migration files. Timestamps start at `m20260515_000000` (or whatever the next slot is at implementation time). Pattern mirrors [migration/src/m20260511_000000_add_hydrated_at_to_coaching_sessions.rs](migration/src/m20260511_000000_add_hydrated_at_to_coaching_sessions.rs).

**`m20260515_000000_add_duration_minutes_to_coaching_sessions.rs`**
- `up`: `ALTER TABLE refactor_platform.coaching_sessions ADD COLUMN duration_minutes SMALLINT NOT NULL DEFAULT 60`
- `down`: `ALTER TABLE ... DROP COLUMN IF EXISTS duration_minutes`

**`m20260515_000001_add_default_coaching_session_duration_minutes_to_users.rs`**
- `up`: `ALTER TABLE refactor_platform.users ADD COLUMN default_coaching_session_duration_minutes SMALLINT NOT NULL DEFAULT 60`
- `down`: `ALTER TABLE ... DROP COLUMN IF EXISTS default_coaching_session_duration_minutes`

Both backfill via the `DEFAULT 60` clause — instant on existing rows. `SMALLINT` saves 2 bytes per row vs `INTEGER` and comfortably accommodates the 1..=480 range.

### Entity Layer

**New: [entity/src/duration.rs](entity/src/duration.rs)** — declares the `Duration` newtype and its associated items. Follows the precedent set by [entity/src/provider.rs](entity/src/provider.rs).

```rust
pub const MIN_DURATION_MINUTES: u16 = 1;
pub const MAX_DURATION_MINUTES: u16 = 480;
pub const DEFAULT_DURATION_MINUTES: u16 = 60;

/// A validated coaching-session duration in minutes (1..=480).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duration(u16);

impl Duration {
    pub fn new(minutes: u16) -> Result<Self, Error> { /* range check */ }
    pub const fn from_minutes_unchecked(minutes: u16) -> Self { Self(minutes) }
    pub const fn minutes(self) -> u16 { self.0 }
}

impl TryFrom<u16> for Duration {
    type Error = Error;
    fn try_from(v: u16) -> Result<Self, Self::Error> { Self::new(v) }
}

impl std::fmt::Display for Duration { /* all 8 singular/plural cases */ }
```

Unit tests live in this module (see Tests section below).

**[entity/src/lib.rs](entity/src/lib.rs)**
- Add `pub mod duration;` alongside the existing `pub mod provider;` and the other module declarations.

**Re-export chain through the layers** (matches the documented pattern at [domain/src/lib.rs:1-6](domain/src/lib.rs) — "consumers of the `domain` crate do not need to directly depend on the `entity_api` crate"):

- **[entity_api/src/lib.rs](entity_api/src/lib.rs)**: extend the existing `pub use entity::{...};` block at [lines 5-10](entity_api/src/lib.rs#L5) to include `duration`. After this edit, internal entity_api code uses `crate::duration::Duration` (just like it uses `crate::provider::Provider` today), and external callers see `entity_api::duration::Duration`.
- **[domain/src/lib.rs](domain/src/lib.rs)**: extend the existing `pub use entity_api::{...};` block at [lines 13-18](domain/src/lib.rs#L13) to include `duration`. After this edit, internal domain code uses `crate::duration::Duration`, and external callers see `domain::duration::Duration`.
- **Web layer**: imports as `domain::duration::Duration` — never reaches across into entity_api or entity directly.

No layer-bypassing imports anywhere.

**[entity/src/coaching_sessions.rs](entity/src/coaching_sessions.rs)**
- Add `pub duration_minutes: u16` to the `Model` struct. Doc comment `/// Session duration in minutes (1..=480).` renders in OpenAPI. Field is `u16` (primitive at storage boundary); validated `Duration` wraps it at the API boundary.

**[entity/src/users.rs](entity/src/users.rs)**
- Add `pub default_coaching_session_duration_minutes: u16` to the `Model` struct. Same doc-comment treatment.

If SeaORM's `u16` mapping to `SMALLINT` proves problematic during implementation (worth a quick smoke check before committing the entity edits), fall back to `i16` in the Model and let the `Duration` newtype handle the conversion at the boundary. The application-facing API uses `u16` (via the newtype) exclusively.

### Entity API Layer

The `Duration` type itself is in entity (see above). Entity_api consumes it and provides the DB-orchestration operations around it.

**[entity_api/src/coaching_session.rs](entity_api/src/coaching_session.rs)** — new public item, importing `use crate::duration::Duration;` (resolves through the re-export in `entity_api/src/lib.rs`):

- `pub async fn resolve_duration(db, coach_id, requested: Option<Duration>) -> Result<Duration, Error>`:
  - `Some(d)` → return `d` directly (already validated by the type).
  - `None` → load coach via `entity_api::user::find_by_id`, wrap their stored default in `Duration::from_minutes_unchecked` (valid by DB invariant).

Existing functions in this module use the new helper:
- `create`: accept additional `requested_duration: Option<Duration>`. Call `resolve_duration`, set `model.duration_minutes = duration.minutes()` before insert.
- `update`: when the update map contains `duration_minutes`, extract the raw value and validate via `Duration::try_from` before passing to `mutate::update`. (The `IntoUpdateMap` pattern erases types, so this is the one site where validation re-runs at the entity_api boundary.)
- `bulk_create_recurring`: accept `requested_duration: Option<Duration>`. Resolve once via `resolve_duration` before iterating — every materialized session gets the same resolved value.

**[entity_api/src/user.rs](entity_api/src/user.rs)** — `use crate::duration::Duration;`. In the update path, when the update map contains `default_coaching_session_duration_minutes`, extract the raw value and validate via `Duration::try_from` before applying the patch.

### Domain Layer

**[domain/src/coaching_session.rs](domain/src/coaching_session.rs)** — `use crate::duration::Duration;` (resolves through the re-export in `domain/src/lib.rs`):
- `create()` ([coaching_session.rs:55-105](domain/src/coaching_session.rs#L55)): change signature to accept `requested_duration: Option<Duration>`. Pass to entity_api's `coaching_session::create` (which internally calls `resolve_duration`).
- `update()` ([coaching_session.rs:219](domain/src/coaching_session.rs#L219)): no signature change — validation happens automatically inside entity_api when the update map carries `duration_minutes`.
- `bulk_create_recurring()`: accept `requested_duration: Option<Duration>`, pass through to entity_api.

**[domain/src/user.rs](domain/src/user.rs)**
- No code change beyond `pub use` re-exports if needed. Validation happens automatically inside entity_api.

**[domain/src/emails.rs](domain/src/emails.rs)**
- `use crate::duration::Duration;` (resolves through the re-export in `domain/src/lib.rs`).
- In `send_session_email_to_recipient` ([emails.rs:348](domain/src/emails.rs#L348)) and `send_recurring_series_email_to_recipient` ([emails.rs:537](domain/src/emails.rs#L537)):
  ```rust
  let session_duration = Duration::from_minutes_unchecked(session.duration_minutes).to_string();
  /* ... */ .add_variable("session_duration", &session_duration)
  ```
- **No standalone `format_duration` helper.** The Display impl on `Duration` is the format helper.

### Web Layer

**[web/src/params/coaching_session/mod.rs](web/src/params/coaching_session/mod.rs)**
- **New** `CreateParams` struct (alongside the existing `UpdateParams` at line 57). Fields mirror `Model` insert fields but with `duration_minutes: Option<u16>`. Pass the optional value through to the domain layer; the entity layer resolves the cascade.
- Update `UpdateParams` (line 57): add `duration_minutes: Option<u16>` field. Extend `IntoUpdateMap` (line 63). Validation happens in entity_api.

**[web/src/params/coaching_session/recurring.rs](web/src/params/coaching_session/recurring.rs)**
- Extend `CreateRecurringParams` (line 11): add `duration_minutes: Option<u16>` field.

**[web/src/params/user/mod.rs](web/src/params/user/mod.rs)**
- Extend `UserParams` (line 12): add `default_coaching_session_duration_minutes: Option<u16>` field. Extend `IntoUpdateMap` (line 22). Validation happens in entity_api.

**[web/src/controller/coaching_session_controller.rs](web/src/controller/coaching_session_controller.rs)**
- `create()` ([line 122](web/src/controller/coaching_session_controller.rs#L122)): change request body type from `Json<Model>` to `Json<CreateParams>`. Use `domain::duration::Duration` (the layer-respecting import path). Convert `params.duration_minutes: Option<u16>` to `Option<Duration>` at the controller boundary:
  ```rust
  use domain::duration::Duration;

  let requested_duration = params.duration_minutes
      .map(Duration::try_from)
      .transpose()?;  // 422 if out of range
  ```
  Pass `requested_duration: Option<Duration>` alongside the converted `Model` into `CoachingSessionApi::create`.
- `create_recurring()` ([line 178](web/src/controller/coaching_session_controller.rs#L178)): same conversion as above, then pass into `bulk_create_recurring`.
- `update()` ([line 223](web/src/controller/coaching_session_controller.rs#L223)): no signature change — `UpdateParams` carries `Option<u16>`. Validation re-runs inside entity_api at the update-map extraction boundary.
- Update `utoipa::path` `request_body` annotations to reference `CreateParams` for the create endpoint.

**[web/src/controller/user_controller.rs](web/src/controller/user_controller.rs)**
- `update()` ([line 61](web/src/controller/user_controller.rs#L61)): no signature change — `UserParams` now carries the new field; validation in entity_api.
- `GET /users/{id}` response automatically includes the new field via the regenerated entity (no controller change needed).

### Tests

**Unit tests in `entity/src/duration.rs`** (newtype lives in entity, so its tests do too)
- `Duration::new` accepts 1, 60, 480; rejects 0, 481, `u16::MAX`.
- `Duration::try_from` mirrors `new`.
- `Duration` `Display` cases — every singular/plural combination:
  - `1` → `"1 minute"`
  - `45` → `"45 minutes"`
  - `60` → `"1 hour"`
  - `61` → `"1 hour 1 minute"`
  - `62` → `"1 hour 2 minutes"`
  - `90` → `"1 hour 30 minutes"`
  - `120` → `"2 hours"`
  - `121` → `"2 hours 1 minute"`
  - `122` → `"2 hours 2 minutes"`
  - `480` → `"8 hours"`

**Integration tests in `entity_api/src/coaching_session.rs`** (operations on top of the type, so tests live alongside the operations)
- `resolve_duration` cascade:
  - `Some(45)` → returns `Duration(45)` (validated direct path).
  - `Some(481)` → returns validation error.
  - `None` and coach default 45 → returns `Duration(45)` (cascade path).
  - `None` and coach default 60 → returns `Duration(60)`.
- `coaching_session::create` with explicit duration produces a row with that duration.
- `coaching_session::create` with `None` and coach default 45 produces a 45-minute session.
- `coaching_session::update` with `duration_minutes = 481` returns a validation error.
- `bulk_create_recurring` with `Some(45)`: every materialized session has `duration_minutes = 45`.
- `bulk_create_recurring` with `None` and coach default 90: every session is 90 minutes.

**Integration tests in `entity_api/src/user.rs`**
- `user::update` with `default_coaching_session_duration_minutes = 481` returns a validation error.
- `user::update` with valid value updates the field.

**Wire-contract tests (must be updated same PR)**
- [emails.rs:1052](domain/src/emails.rs#L1052) `test_send_session_scheduled_email_variables`: add `"session_duration": "<expected>"` to both `match_body` JSON assertions.
- [emails.rs:1492](domain/src/emails.rs#L1492) `test_send_recurring_sessions_scheduled_email_personalization`: same addition.
- Without these updates both tests hang on `expect(1)` because `Matcher::Json` is structural and rejects the extra variable.

**Migration test** — verify post-`up` that existing rows in both tables backfill to 60.

## Resend Templates (Manual Setup)

Update the **session-scheduled** and **recurring-sessions-scheduled** templates in the Resend dashboard to render the new `{{session_duration}}` variable in the email body. Suggested copy fragment:

> "{{session_date}} at {{session_time}} for {{session_duration}}"

Welcome, password-reset, and action-assigned templates are unaffected.

## Verification

### Manual (post-merge, after Resend templates are updated)
1. Schedule a coaching session via the BE API: `POST /coaching_sessions` with `duration_minutes: 45`. Confirm both recipients receive emails containing "for 45 minutes."
2. Schedule a session **without** `duration_minutes` in the body when the coach's `default_coaching_session_duration_minutes = 90`. Confirm both emails say "for 1 hour 30 minutes" and the DB row has `duration_minutes = 90`.
3. Update the coach's `default_coaching_session_duration_minutes` via `PUT /users/{id}` with `481` — confirm 422.
4. Update a session via `PUT /coaching_sessions/{id}` changing only `duration_minutes` — confirm the patch succeeds and other fields are untouched.
5. Create a recurring series via `POST /coaching_sessions/recurring` with `duration_minutes: 30`. Confirm every materialized session has 30 minutes.

### Automated
- `cargo check` — confirms types align across the migration / entity / entity_api / domain / web layers.
- `cargo test -p entity duration` — Duration newtype unit tests (validation + Display).
- `cargo test -p entity_api coaching_session` — cascade + create/update/recurring integration tests.
- `cargo test -p entity_api user` — user-update validation test.
- `cargo test -p domain emails` — wire-contract tests (updated).
- `cargo clippy --workspace -- -D warnings` and `cargo fmt --check` per project standards.

## Risks & Rollback

**No env-passthrough work required.** Per CLAUDE.md's standing rule, new env vars must be wired through `docker-compose.yaml`, `docker-compose.pr-preview.yaml`, `.github/workflows/deploy_to_do.yml`, and `.github/workflows/ci-deploy-pr-preview.yml`, or they silently resolve to empty strings at runtime. **This feature introduces no new env vars** — duration is an intrinsic entity column with a DB default, not an environment-driven setting. The four passthrough files are untouched.

**Migration safety.** Both migrations are pure column-adds with safe defaults; backfill is instant via `DEFAULT 60`. Rolling forward on a populated DB is safe. Rolling back via `down` drops the columns — destructive if the application has started writing non-60 values, but reversible enough during PR-preview testing.

**Wire-contract test cascade.** Two existing tests in `domain/src/emails.rs` must be updated in the same PR or CI will go red. Listed explicitly in Tests above so this isn't a surprise.

**Defaulting cascade adds a DB round trip on omitted-duration creates.** When `duration_minutes` is omitted, we do an extra `user::find_by_id(db, coach_id)` to read the coach's default. The relationship lookup at [coaching_session.rs:60-62](domain/src/coaching_session.rs#L60) doesn't fetch the coach, so this is one additional SELECT per omitted-duration create. Acceptable; can be optimized later via a JOIN if it becomes a hot path.

**SeaORM `u16` mapping uncertainty.** Verify during implementation that SeaORM cleanly maps `u16` to `SMALLINT`. If not, fall back to `i16` in the entity Model and have the `Duration` newtype handle the conversion. Either way, the application-facing API uses `u16` exclusively via the newtype.

**Frontend coordination.** FE will file a paired issue covering the schedule/reschedule dialog + user-settings UI. Per the contract `CoachingSessionDurationFeature v1` on the collab board, FE can rely on the defaulting cascade — omitting `duration_minutes` from the payload still produces a valid session. BE ships independently.

**Future extraction of `coach_settings` table.** If 3+ coach-specific preferences land in the future, extracting `default_coaching_session_duration_minutes` from `users` into a dedicated `coach_settings` table is a clean future refactor — well-defined, single-purpose, no behavior change for callers. Not in scope for v1; flagged as a known evolution path.

## PR Breakdown

**Recommendation: single PR.** The changes are tightly coupled (migration is useless without entity regen, entity is useless without validation, validation is useless without controller wire-up). Splitting introduces awkward intermediate states.

If the team prefers a split for review-surface reasons:
- **PR 1:** Two migrations + entity regen. Deployable as a no-op (no code reads the fields yet).
- **PR 2:** Entity_api Duration newtype + validation + resolve_duration + domain orchestration + web layer + tests.

The single-PR option is cleaner. The two-PR split is acceptable but should not delay #333.
