# Coaching Session `display_title` — Backend Implementation Plan

## Goal

Add a server-composed `display_title: string | null` field to the two coaching-session
**list reads** so every list/preview surface derives an identical session title without
shipping topic arrays or re-implementing the fallback chain client-side.

Settles the coordinator `session_title_topics_include` question (Option B, BE composes the
chain), FE-confirmed 2026-06-18.

### The fallback chain (BE owns it now)

```
human-set title  →  first topic body  →  first goal title  →  null
```

- No synthetic `"Coaching Session"` default. When no tier yields text, `display_title` is `null`.
- `null` (not `""`) is the no-value encoding — matches the sibling `title` field (already
  `string | null`, never `""` on the wire) and serde `Option<String>` default. The FE renders
  `display_title ?? <fe placeholder>`; the placeholder stays an FE presentation choice.
- **Why `null` matters for the FE component (clarified):** returning `null` (not a baked default)
  is exactly what lets the FE title component expose the fallback as a **per-call-site parameter**
  (e.g. a `defaultTitle` prop). Each surface overrides it independently — "Coaching Session" on the
  dashboard, a different label elsewhere — and no surface ever renders `""`. A BE-baked default
  would force one word everywhere; `null` keeps the choice at the call site. This is an FE concern
  (no BE change), recorded here so the rationale for `null` isn't mistaken for an oversight.

### Scope (narrowed by FE confirmation — read before building)

- **IN:** enriched `GET /users/{user_id}/coaching_sessions` AND the shared relationship-scoped
  `GET /coaching_sessions?coaching_relationship_id=`.
- **OUT:** the single-session `GET /coaching_sessions/{id}`. The FE explicitly declined it — the
  single page keeps client-side derivation so it stays consistent with optimistic title/topic
  edits a server field would lag behind. Do NOT add the field there.
- Always-present field (value may be `null`), **not** include-gated. Additive. No SSE. No new
  endpoint. **No change to any topic or goal endpoint.**

## Key architectural decision

The two target endpoints differ in return type and query path:

| Endpoint | Handler | entity_api fn | Return type today |
|---|---|---|---|
| Enriched user-list | `web/src/controller/user/coaching_session_controller.rs::index` | `find_by_user_with_includes` | `Vec<EnrichedSession>` |
| Relationship-scoped list | `web/src/controller/coaching_session_controller.rs::index` | `CoachingSessionApi::find_by` (generic `query::find_by`) | `Vec<coaching_sessions::Model>` |

**Do NOT reuse `EnrichedSession` for the relationship-scoped endpoint.** `EnrichedSession`
carries `viewer_last_viewed_at`, which is **caller-scoped**; the `CoachingSessionViews` v2
contract deliberately kept that off the shared relationship-scoped list. `display_title`, by
contrast, is **not** caller-scoped (same value for coach and coachee), so it is safe on the
shared list — but it must travel on its own minimal wrapper, not by importing the view marker.

Therefore: factor the title logic into a **shared composition primitive** + **two lightweight
batch loaders**, consumed by both assembly sites independently.

## Design — shared primitives (in `entity_api/src/coaching_session_display_title.rs`)

> **Module placement (decided during build):** the composition + loaders live in their
> own sibling module `entity_api/src/coaching_session_display_title.rs`, mirroring
> `coaching_session_topic.rs` / `coaching_session_goal.rs` / `coaching_session_view.rs`.
> It is a *module*, not a new DB entity — `display_title` is a read-time projection with
> no stored state, identity, or lifecycle (the opposite of `coaching_session_views`, which
> earned its own table). `batch_load_display_titles` is `pub` so both the enriched path
> (entity_api) and the relationship-scoped path (domain) consume it; the inner loaders +
> `compose_display_title` stay private/`pub(crate)`.

### 1. Pure composition function

```rust
/// Compose the display title from the fallback chain. Treats empty / whitespace-only
/// inputs as absent so a blank topic body or goal title falls through to the next tier.
fn compose_display_title(
    session_title: Option<&str>,
    first_topic_body: Option<&str>,
    first_goal_title: Option<&str>,
) -> Option<String> {
    [session_title, first_topic_body, first_goal_title]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_owned)
}
```

- Pure, trivially unit-testable, no DB. The emptiness guard matters: goal `title` can be `""`
  (see `ActiveGoalLimitConflict` note), and `title` is already whitespace-normalized on write
  but we trim defensively.

### 2. Two lightweight batch loaders (mirror `batch_load_views`)

```rust
/// First (drag-order) live topic body per session. Reuses the EXACT canonical ordering +
/// soft-delete filter from `coaching_session_topic::find_by_coaching_session_id`, so it
/// inherits v5 move/defer parenting (CoachingSessionId = this session) and v6 soft-delete
/// exclusion (DeletedAt IS NULL) by construction.
async fn batch_load_first_topic_bodies(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, String>, Error>;

/// First linked goal title per session. Reuses `coaching_session_goal::find_goals_grouped_by_session_ids`
/// and takes the first goal's title.
async fn batch_load_first_goal_titles(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, String>, Error>;
```

Implementation notes:
- `batch_load_first_topic_bodies`: one query — `coaching_session_topics` where
  `CoachingSessionId IN (...)` AND `DeletedAt IS NULL`, ordered by `DisplayOrder ASC, CreatedAt ASC`
  (the canonical order). Select only the columns needed (`coaching_session_id`, `body`,
  `display_order`, `created_at`) and reduce to the first row per session in Rust (keep the first
  seen per `coaching_session_id` since the result is already globally ordered, or group then pick
  min). Matches the existing `batch_load_topics` shape but lighter (no full Model vec, `LIMIT`-1
  semantics per session).
- `batch_load_first_goal_titles`: call the existing grouped-goals loader and `.first()` each vec.
- Optional optimization (non-blocking): both could be a single SQL `DISTINCT ON (coaching_session_id)`
  / lateral query. Start with the Rust-reduce version for consistency with existing `batch_load_*`
  code; revisit only if profiling warrants.

### 3. One thin helper to assemble a title map

```rust
/// Compose display titles for a set of sessions in one place. `titles` is each session's own
/// `title` column keyed by id (callers already hold the Models).
async fn batch_load_display_titles(
    db: &impl ConnectionTrait,
    sessions: &[(Id, Option<String>)],   // (session_id, session.title)
) -> Result<HashMap<Id, Option<String>>, Error>;
```

Loads first-topic-bodies + first-goal-titles for the ids, then `compose_display_title` per
session. Both call sites use this; the composition rule lives exactly once.

## Wiring — site A: enriched user-list (`EnrichedSession`)

1. Add field to `EnrichedSession` (entity_api/src/coaching_session.rs, near line 494, beside
   `viewer_last_viewed_at`):
   ```rust
   // Server-composed fallback title (human title → first topic body → first goal title);
   // null when none derive. Always present.
   pub display_title: Option<String>,
   ```
   **No `#[serde(skip_serializing_if)]`** — must serialize as `null` when absent (same as
   `viewer_last_viewed_at`).
2. In `find_by_user_with_includes`: after the base sessions are fetched, call
   `batch_load_display_titles` **unconditionally** (like `batch_load_views`), independent of the
   `include` flags. Populate `display_title` in both the includes-assembly path
   (`assemble_enriched_session`) and the no-includes path (where `viewer_last_viewed_at` is set
   per session today).
3. Reuse already-loaded data when present: if `include=topics`/`include=goal` already loaded the
   full vecs, prefer composing from those to avoid a redundant query. Keep it simple — correctness
   first; the extra two batch queries are cheap and keyed on the same id set.

## Wiring — site B: relationship-scoped list (plain `Model` today)

1. Introduce a minimal serialized wrapper (entity_api/src/coaching_session.rs):
   ```rust
   #[derive(Debug, Clone, serde::Serialize, ToSchema)]
   #[schema(as = domain::coaching_session::SessionWithDisplayTitle)]
   pub struct SessionWithDisplayTitle {
       #[serde(flatten)]
       pub session: coaching_sessions::Model,
       pub display_title: Option<String>,   // always present; null when none
   }
   ```
   Deliberately does **not** include `viewer_last_viewed_at` (caller-scoped; out of scope here).
2. The relationship-scoped `index` controller currently returns `Vec<Model>` from
   `CoachingSessionApi::find_by`. Two options:
   - **B1 (preferred):** add a domain fn `find_by_with_display_title(db, params)` that runs the
     existing `find_by`, then maps the Models through `batch_load_display_titles`, returning
     `Vec<SessionWithDisplayTitle>`. Controller switches to it. Smallest blast radius; the generic
     `find_by` filter path is untouched.
   - **B2:** enrich inside the controller. Avoid — keeps composition logic out of entity_api.
3. Update the utoipa `responses(... body = [coaching_sessions::Model])` to the new wrapper type.

## Contract + versioning (owed after build)

- Bump `UserCoachingSessionsListEndpoint` (add `display_title` to the enriched read shape).
- Bump `CoachingSessionsListEndpoint` → v3. **Coordinate with the already-promised v3** for that
  endpoint (the date-filter-fix the contract says "v3 will be posted with the fix"); fold
  `display_title` into the same bump or sequence them clearly.
- Cross-reference `CoachingSessionTitleField` (the `title` field this falls back from).
- Post to the coordinator board when **built + verified live**, per the FE's "ping the board"
  request. Until then the FE types against the agreed shape.

## Testing

Mock-gated suite (per project convention):
`cargo test -p entity_api -p domain -p web --features "domain/mock,web/mock"`.
Frozen-test discipline: unit tests in a separate `*_tests.rs` wired via
`#[cfg(test)] #[path = "..."] mod tests;`, never an in-file `mod tests`.

- **Pure `compose_display_title`** (no DB): title wins over topic/goal; empty/whitespace title
  falls through to topic; empty topic body falls through to goal; all-empty → `None`; goal `""`
  treated as absent.
- **`batch_load_first_topic_bodies`**: returns drag-order-first body; excludes soft-deleted
  (`DeletedAt` set) topics; ignores topics parented to other sessions (v5 move semantics); empty
  when session has no live topics.
- **`batch_load_first_goal_titles`**: first linked goal's title; none when no linked goals.
- **End-to-end per endpoint** (web/mock where feasible): enriched list returns `display_title`
  matching the chain; relationship-scoped list returns the wrapper with `display_title`; `null`
  surfaces when nothing derives; field present even with no `include=`.

Run `cargo clippy` and `cargo fmt` before committing. No `.unwrap()` in production paths.

## Phases (single coherent PR; sequence for clean review)

1. **Primitives** — `compose_display_title` (+ unit tests), `batch_load_first_topic_bodies`,
   `batch_load_first_goal_titles`, `batch_load_display_titles` (+ entity_api tests).
2. **Site A** — `display_title` on `EnrichedSession` + unconditional load in
   `find_by_user_with_includes`; assembly in both include/no-include paths; tests.
3. **Site B** — `SessionWithDisplayTitle` wrapper + `find_by_with_display_title`; relationship-scoped
   controller + utoipa response type; tests.
4. **Contracts + verify** — live-verify both endpoints on real Postgres; bump the two list-endpoint
   contracts; post the board update.

## Out of scope / explicit non-goals

- Single-session `GET /coaching_sessions/{id}` (FE keeps client-side derivation there).
- Any change to topic/goal endpoints, enums, or wire shapes.
- SSE (none; title is derived, not a live event).
- No migration — `display_title` is computed, not stored.

## Branch

New branch off `main` (e.g. `feat/coaching-session-display-title`). Additive; no coordinated
deploy needed — the field is read-only-when-present and the FE consumes it via its existing
`CoachingSessionTitleText` seam once it appears.
