# Goals Feature — Backend Implementation Plan

## Context

The frontend team is building a goals feature that transforms overarching goals from per-session text labels into relationship-level entities tracked across multiple sessions. The backend needs schema changes, new endpoints, SSE events, and a foundational rename. This plan covers 5 PRs that correspond to the 4 frontend questions on the coordination board, plus a preparatory rename.

The rename from `overarching_goals` → `goals` is motivated by the broadening semantics: goals are no longer just "overarching" — they can be general, localized, or aspirational. Doing the rename first avoids carrying the verbose name into all new code.

---

## PR 1 / Milestone 1: Rename `overarching_goals` → `goals`

**Goal:** Pure rename across all layers — no behavior change, no new features.

### Database Migration
- New migration: `m20260228_000000_rename_overarching_goals_to_goals.rs`
- `ALTER TABLE refactor_platform.overarching_goals RENAME TO goals;`
- `ALTER TABLE refactor_platform.goals OWNER TO refactor;`
- Down migration: rename back

### Files to Rename (9 files)
| From | To |
|------|-----|
| `entity/src/overarching_goals.rs` | `entity/src/goals.rs` |
| `entity_api/src/overarching_goal.rs` | `entity_api/src/goal.rs` |
| `domain/src/overarching_goal.rs` | `domain/src/goal.rs` |
| `web/src/controller/overarching_goal_controller.rs` | `web/src/controller/goal_controller.rs` |
| `web/src/controller/user/overarching_goal_controller.rs` | `web/src/controller/user/goal_controller.rs` |
| `web/src/protect/overarching_goals.rs` | `web/src/protect/goals.rs` |
| `web/src/protect/users/overarching_goals.rs` | `web/src/protect/users/goals.rs` |
| `web/src/params/overarching_goal.rs` | `web/src/params/goal.rs` |
| `web/src/params/user/overarching_goal.rs` | `web/src/params/user/goal.rs` |

### Module Declarations to Update
- `entity/src/lib.rs` — `pub mod overarching_goals` → `pub mod goals`
- `entity_api/src/lib.rs` — re-export + `pub mod overarching_goal` → `pub mod goal`
- `domain/src/lib.rs` — re-export + `pub mod overarching_goal` → `pub mod goal`
- `web/src/controller/mod.rs` — `pub(crate) mod overarching_goal_controller` → `pub(crate) mod goal_controller`
- `web/src/controller/user/mod.rs` — same pattern
- `web/src/protect/mod.rs` — `pub(crate) mod overarching_goals` → `pub(crate) mod goals`
- `web/src/protect/users/mod.rs` — same pattern
- `web/src/params/mod.rs` — `pub(crate) mod overarching_goal` → `pub(crate) mod goal`
- `web/src/params/user/mod.rs` — same pattern

### API Routes (`web/src/router.rs`)
- `/overarching_goals` → `/goals`
- `/overarching_goals/:id` → `/goals/:id`
- `/overarching_goals/:id/status` → `/goals/:id/status`
- `/users/:user_id/overarching_goals` → `/users/:user_id/goals`
- Rename route builder functions: `overarching_goal_routes()` → `goal_routes()`, etc.
- Update OpenAPI path registrations and schema references

### SSE Events
- `sse/src/message.rs`: `OverarchingGoalCreated/Updated/Deleted` → `GoalCreated/Updated/Deleted`
- Serde renames: `overarching_goal_created` → `goal_created`, etc.
- Field names: `overarching_goal: Value` → `goal: Value`, `overarching_goal_id` → `goal_id`
- `events/src/lib.rs`: Same renames on `DomainEvent` variants
- `sse/src/domain_event_handler.rs`: Update match arms

### Domain Layer
- `domain/src/goal.rs` (renamed): Update import alias `OverarchingGoalApi` → `GoalApi`
- `domain/src/emails.rs`: Update `use crate::overarching_goal` → `use crate::goal`, field names in `ActionAssignmentContext`, MailerSend personalization keys (`overarching_goal` → `goal`)

### Entity Layer
- `entity/src/goals.rs` (renamed): Update `#[sea_orm(table_name = "goals")]`, relation names
- `entity/src/coaching_sessions.rs`: Update `has_many` relation to reference `goals::Entity`

### Additional Files with References to Update
- `entity_api/src/coaching_session.rs`: Update `use entity::overarching_goals` → `use entity::goals`
- `migration/src/m20250801_000000_add_sorting_indexes.rs`: Update index references if any use `overarching_goals` table name
- `docs/db/refactor_platform_rs.dbml`: Update table name and references
- `docs/db/base_refactor_platform_rs.dbml`: Update table name and references
- `.claude/coding-standards.md`: Update any references to overarching_goals
- Update all code comments referencing "overarching goal(s)" → "goal(s)"

### Coordination
- **Frontend must update simultaneously**: API paths `/overarching_goals` → `/goals`, SSE event names `overarching_goal_*` → `goal_*`
- **MailerSend templates**: Check if `overarching_goal` personalization key is used in templates — may need template update

---

## PR 2 / Milestone 2: Goal Scoping — Join Table + Relationship FK (Q1, Option B)

**Goal:** Add `coaching_relationship_id` to goals and create `coaching_sessions_goals` join table.

### Database Migration
- New migration: `m20260XXX_000000_add_goal_scoping.rs`
- **Step 1a**: Add `coaching_relationship_id UUID` column to `goals` (initially nullable for the data migration)
  - FK constraint → `coaching_relationships(id)`, ON DELETE CASCADE
- **Step 1b**: Add `target_date DATE NULL` column to `goals` — optional intended achieve-by date for dynamic health signal computation
- **Step 2**: Data migration — populate `coaching_relationship_id` for all existing goals:
  ```sql
  UPDATE refactor_platform.goals g
  SET coaching_relationship_id = cs.coaching_relationship_id
  FROM refactor_platform.coaching_sessions cs
  WHERE g.coaching_session_id = cs.id;
  ```
  This follows each goal's existing `coaching_session_id` to look up which coaching relationship the session belongs to, then stores that relationship ID directly on the goal.
- **Step 3**: Set column to `NOT NULL` after data migration (all rows now populated)
- **Step 4**: Rename `coaching_session_id` → `created_in_session_id` on goals and make it **nullable** — clarifies its new meaning (originating session, not current association) and allows goals to be created outside of a session context (e.g. from the dashboard or goals page)
- **Step 5**: Create `coaching_sessions_goals` join table — allows a goal to be associated with any number of sessions:
  - `id UUID PRIMARY KEY DEFAULT gen_random_uuid()`
  - `coaching_session_id UUID NOT NULL` (FK → `coaching_sessions`, CASCADE)
  - `goal_id UUID NOT NULL` (FK → `goals`, CASCADE)
  - `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`
  - `updated_at TIMESTAMPTZ NOT NULL DEFAULT now()`
  - Unique index on `(coaching_session_id, goal_id)` — prevent duplicate links
  - Index on `coaching_session_id` and `goal_id` — efficient lookups from either side
  - `ALTER TABLE ... OWNER TO refactor`
- **Step 6**: Seed join table from existing data — ensures every existing goal remains linked to its original session:
  ```sql
  INSERT INTO refactor_platform.coaching_sessions_goals (coaching_session_id, goal_id)
  SELECT created_in_session_id, id FROM refactor_platform.goals;
  ```

### Entity Layer
- New file: `entity/src/coaching_sessions_goals.rs` — SeaORM model following `actions_users` pattern
- Update `entity/src/goals.rs`:
  - Add `coaching_relationship_id: Id` field
  - Add `target_date: Option<Date>` field — optional intended achieve-by date
  - Rename `coaching_session_id` → `created_in_session_id: Option<Id>` (nullable)
  - Add `has_many` relation to `CoachingSessionsGoals`
  - Add `belongs_to` relation to `CoachingRelationships`
- Update `entity/src/coaching_sessions.rs`: Add `has_many` relation to `CoachingSessionsGoals`
- Update `entity/src/coaching_relationships.rs`: Add `has_many` relation to `Goals`
- Register `coaching_sessions_goals` in `entity/src/lib.rs`

### Entity API Layer
- New file: `entity_api/src/coaching_session_goal.rs` — CRUD following `actions_users` pattern:
  - `create(db, session_id, goal_id)` → Model
  - `delete(db, session_id, goal_id)` → ()
  - `find_by_coaching_session_id(db, session_id)` → Vec<Model>
  - `find_by_goal_id(db, goal_id)` → Vec<Model>
  - `find_goal_ids_by_session_id(db, session_id)` → Vec<Id>
  - `find_session_ids_by_goal_id(db, goal_id)` → Vec<Id>
- Update `entity_api/src/goal.rs`: Add `find_by_coaching_relationship_id()` query

### Domain Layer
- New file: `domain/src/coaching_session_goal.rs` — wraps entity_api, publishes SSE events
- Update `domain/src/goal.rs`: Update create to accept `coaching_relationship_id`

### Web Layer
- New controller: `web/src/controller/coaching_session_goal_controller.rs`
  - `POST /coaching_session_goals` — link goal to session
  - `DELETE /coaching_session_goals/:id` — unlink goal from session
  - `GET /coaching_sessions/:session_id/goals` — goals for a session
  - `GET /goals/:goal_id/sessions` — sessions for a goal
- New protect middleware: `web/src/protect/coaching_session_goals.rs` — follow existing protect pattern closely (verify user is coach/coachee of the relationship via session lookup before allowing access)
- New params: `web/src/params/coaching_session_goal.rs`
- Update `web/src/params/goal.rs`: Add `coaching_relationship_id` filter to IndexParams
- Update `web/src/params/user/goal.rs`: Add `coaching_relationship_id` filter
- Register routes in `web/src/router.rs`

---

## PR 3 / Milestone 3: Action FK to Goals (Q2, Option A — confirmed)

**Goal:** Add nullable `goal_id` FK to actions so actions can be directly associated with a goal.

**How `coaching_session_id` and `goal_id` coexist:** These answer different questions. `coaching_session_id` = "which session was this action created in?" (session context — always set). `goal_id` = "which goal is this action working toward?" (strategic intent — nullable, since not every action is tied to a goal). A goal spans many sessions, so actions from different sessions can all point to the same goal. Example: Session A creates Action 1 for Goal X (`session=A, goal=X`), Session B creates Action 2 also for Goal X (`session=B, goal=X`), and Session B also creates Action 3 with no goal (`session=B, goal=NULL`).

### Database Migration
- New migration: `m20260XXX_000000_add_goal_id_to_actions.rs`
- `ALTER TABLE refactor_platform.actions ADD COLUMN goal_id UUID NULL`
- FK constraint → `goals(id)` with `ON DELETE SET NULL` (deleting a goal shouldn't delete actions)
- Index on `goal_id` for efficient queries

### Entity Layer
- Update `entity/src/actions.rs`: Add `goal_id: Option<Id>` field, add `belongs_to` relation to `Goals`
- Update `entity/src/goals.rs`: Add `has_many` relation to `Actions`

### Entity API Layer
- Update `entity_api/src/action.rs`: Include `goal_id` in create/update operations
- Update query params to support `goal_id` filtering in `find_by` queries

### Web Layer
- Update `web/src/params/action.rs` `IndexParams`: Add optional `goal_id` filter
- Update `web/src/params/user/action.rs`: Add optional `goal_id` filter
- No new endpoints needed — existing `GET /actions?goal_id=X` works via generic query pattern
- Update controller create/update to accept `goal_id` in request body

---

## PR 4 / Milestone 4: SSE Events + Health Signals (Q3)

**Goal:** Add SSE events for the join table and backend-computed health signals.

### New SSE Events (for join table from PR2)
- `events/src/lib.rs`: Add `DomainEvent` variants:
  - `CoachingSessionGoalCreated { coaching_session_id, goal_id, notify_user_ids }`
  - `CoachingSessionGoalDeleted { coaching_session_id, goal_id, notify_user_ids }`
- `sse/src/message.rs`: Add `Event` variants:
  - `CoachingSessionGoalCreated { coaching_relationship_id, coaching_session_id, goal_id }`
  - `CoachingSessionGoalDeleted { coaching_relationship_id, coaching_session_id, goal_id }`
  - Serde renames: `coaching_session_goal_created`, `coaching_session_goal_deleted`
- `sse/src/domain_event_handler.rs`: Add match arms
- Update `domain/src/coaching_session_goal.rs`: Publish events on create/delete

### Health Signals (synchronous, computed on read)
- New enum in `entity/src/goals.rs` or a shared location:
  ```rust
  enum GoalHealth {
      SolidMomentum,
      NeedsAttention,
      LetsRefocus,
  }
  ```
- New response struct `GoalHealthMetrics`:
  - `actions_completed: i32`
  - `actions_total: i32`
  - `linked_session_count: i32`
  - `health: GoalHealth`
  - `last_session_date: Option<Date>`
  - `next_action_due: Option<DateTimeWithTimeZone>`
- `entity_api/src/goal.rs`: Add `compute_health_metrics(db, goal_id)` — queries actions + sessions
- `domain/src/goal.rs`: Expose health computation
- `web/src/controller/goal_controller.rs`: New endpoint `GET /goals/:id/health` or enrich goal responses
- Health computation logic — **dynamic, based on goal's `target_date`** when set:
  - When `target_date` is set, compute `elapsed_pct` = (now - created_at) / (target_date - created_at) and `progress_pct` = actions_completed / actions_total
  - `SolidMomentum`: `progress_pct >= elapsed_pct` (on track or ahead of schedule) AND at least one session discussed it recently (within 25% of remaining duration or 2 weeks, whichever is shorter)
  - `NeedsAttention`: `progress_pct < elapsed_pct` (falling behind) OR no recent session within the threshold above
  - `LetsRefocus`: `progress_pct < elapsed_pct * 0.5` (significantly behind — less than half the expected progress) OR no session in 50%+ of remaining duration
  - When `target_date` is NULL, base health purely on **action progress + recency of activity** (no time-based deadlines):
    - `SolidMomentum`: Actions are being completed regularly (completion rate trending up or steady)
    - `NeedsAttention`: Action completion has stalled (no actions completed recently despite open actions remaining)
    - `LetsRefocus`: No action progress at all (zero actions completed, or no activity for an extended period)
  - Goals without a `target_date` have no timeline pressure — health reflects momentum, not deadlines
  - These heuristics are refineable — the key insight is that health is relative to the goal's intended timeline when one is set

---

## PR 5 / Milestone 5: Entity Validation Endpoint (Q4, Option C — confirmed)

**Goal:** Add per-entity-type batch validation endpoints for stale TipTap mark cleanup. Combined with existing SSE deletion events for real-time cleanup (comes free — no backend work needed for the SSE part).

### Approach
Per-entity-type endpoints (stays consistent with controller-per-entity pattern):
- `POST /actions/validate` — check which action IDs still exist
- `POST /agreements/validate` — check which agreement IDs still exist
- `POST /goals/validate` — check which goal IDs still exist

### Request/Response Contract
```rust
// Request body for all three endpoints
struct ValidateRequest {
    ids: Vec<Id>,  // UUIDs to check
}

// Response for all three endpoints
struct ValidateResponse {
    valid: Vec<Id>,
    invalid: Vec<Id>,
}
```

### Entity API Layer
- Add generic `validate_ids<E: EntityTrait>(db, ids: Vec<Id>) -> (Vec<Id>, Vec<Id>)` utility
  - Query `SELECT id FROM table WHERE id IN (...)`
  - Compare returned IDs vs input to determine valid/invalid
- Or add `validate_ids()` to each entity_api module individually

### Web Layer
- Add `validate` handler to `action_controller.rs`, `agreement_controller.rs`, `goal_controller.rs`
- Route: `POST /actions/validate`, `POST /agreements/validate`, `POST /goals/validate`
- Authorization: User must have access to the coaching relationship containing these entities
  - Validate by checking entity ownership/relationship membership

### SSE Part (comes free)
- Frontend uses existing deletion events (`goal_deleted`, `action_deleted`, `agreement_deleted`) to clean up marks in real-time — no backend work needed for this part

---

## Execution Approach

Work proceeds **one PR/milestone at a time**, in order. Each PR is a complete, reviewable unit — fully implemented, tested, and merged before starting the next.

## Dependency Order

```
PR1 (rename) → PR2 (join table + relationship FK) → PR3 (action FK)
                                                          ↓
                                              PR4 (SSE events + health)
                                                          ↓
                                              PR5 (entity validation)
```

PR1 must land first. PR2 and PR3 could theoretically be parallel but PR4 depends on both. PR5 depends on PR4 only loosely (same branch of work).

---

## Endpoint Reference (All Milestones)

### PR1 — Renamed Endpoints (behavior unchanged)

| Method | Old Path | New Path |
|--------|----------|----------|
| POST | `/overarching_goals` | `/goals` |
| GET | `/overarching_goals` | `/goals` |
| GET | `/overarching_goals/:id` | `/goals/:id` |
| PUT | `/overarching_goals/:id` | `/goals/:id` |
| PUT | `/overarching_goals/:id/status` | `/goals/:id/status` |
| GET | `/users/:user_id/overarching_goals` | `/users/:user_id/goals` |

### PR2 — New Endpoints (join table + scoping)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/coaching_session_goals` | Link a goal to a coaching session |
| DELETE | `/coaching_session_goals/:id` | Unlink a goal from a coaching session |
| GET | `/coaching_sessions/:session_id/goals` | List all goals linked to a session |
| GET | `/goals/:goal_id/sessions` | List all sessions linked to a goal |

**Updated query params on existing endpoints:**
- `GET /goals?coaching_relationship_id=X` — filter goals by relationship
- `GET /goals?status=InProgress` — filter goals by status
- `GET /users/:user_id/goals?coaching_relationship_id=X` — same filters on user-scoped endpoint

### PR3 — Updated Endpoints (action FK)

No new endpoints. Existing endpoints gain `goal_id` support:

| Method | Path | Change |
|--------|------|--------|
| POST | `/actions` | Request body accepts optional `goal_id` |
| PUT | `/actions/:id` | Request body accepts optional `goal_id` |
| GET | `/actions?goal_id=X` | New filter param — actions for a specific goal |
| GET | `/users/:user_id/actions?goal_id=X` | Same filter on user-scoped endpoint |

### PR4 — New Endpoints (health signals)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/goals/:id/health` | Returns `GoalHealthMetrics` with health signal, action stats, session stats |

**New SSE event types:**
- `coaching_session_goal_created` — fires when a goal is linked to a session
- `coaching_session_goal_deleted` — fires when a goal is unlinked from a session

### PR5 — New Endpoints (entity validation)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/actions/validate` | Batch check which action IDs exist |
| POST | `/agreements/validate` | Batch check which agreement IDs exist |
| POST | `/goals/validate` | Batch check which goal IDs exist |

All three use the same request/response contract: `{ ids: [uuid, ...] }` → `{ valid: [...], invalid: [...] }`

---

## Test Updates and New Tests

Tests use `MockDatabase` with `#[cfg(feature = "mock")]`. Run with `cargo test --features mock`.

### PR1 (Rename)
**Existing tests to update (rename references only, no logic changes):**
- `entity_api/src/overarching_goal.rs` → `entity_api/src/goal.rs` — 4 tests: rename model references, function names, variable names
- `entity_api/src/coaching_session.rs` — 13 tests: update `overarching_goals` import references
- `domain/src/emails.rs` — update `overarching_goal` field references in test assertions
- `domain/src/overarching_goal.rs` → `domain/src/goal.rs` — update test module references

### PR2 (Join Table + Scoping)
**New tests to add:**
- `entity_api/src/coaching_session_goal.rs` — follow `actions_user.rs` test pattern (6+ tests):
  - `create_returns_a_new_coaching_session_goal`
  - `delete_removes_coaching_session_goal`
  - `find_by_coaching_session_id_returns_linked_goals`
  - `find_by_goal_id_returns_linked_sessions`
  - `find_goal_ids_by_session_id_returns_ids`
  - `find_session_ids_by_goal_id_returns_ids`
- `entity_api/src/goal.rs` — add test for `find_by_coaching_relationship_id()`
- Update existing goal tests: mock models need `coaching_relationship_id` and `target_date` fields, `coaching_session_id` → `created_in_session_id`

### PR3 (Action FK)
**Existing tests to update:**
- `entity_api/src/action.rs` — 13 tests: all mock action models need `goal_id: None` added
- Update `find_by` tests to verify `goal_id` filtering works
**New tests to add:**
- `find_by_with_goal_id_filter` — verify `GET /actions?goal_id=X` returns filtered results

### PR4 (SSE + Health)
**New tests to add:**
- `entity_api/src/goal.rs` — health computation tests:
  - `compute_health_with_target_date_on_track` → SolidMomentum
  - `compute_health_with_target_date_behind` → NeedsAttention
  - `compute_health_with_target_date_stalled` → LetsRefocus
  - `compute_health_without_target_date_active` → SolidMomentum
  - `compute_health_without_target_date_stalled` → NeedsAttention
  - `compute_health_without_target_date_no_progress` → LetsRefocus
- `testing-tools/src/scenarios.rs` — integration test scenarios for new SSE events:
  - `test_coaching_session_goal_create` — verify `coaching_session_goal_created` event fires
  - `test_coaching_session_goal_delete` — verify `coaching_session_goal_deleted` event fires

### PR5 (Validation)
**New tests to add:**
- `entity_api/src/action.rs` — `validate_ids_returns_valid_and_invalid`
- `entity_api/src/agreement.rs` — `validate_ids_returns_valid_and_invalid`
- `entity_api/src/goal.rs` — `validate_ids_returns_valid_and_invalid`

## Verification

For each PR:
1. `cargo fmt` — formatting
2. `cargo clippy` — linting
3. `cargo build` — compilation
4. `cargo test` — unit tests pass
5. Manual testing against local PostgreSQL with migrations applied
6. For PR1: verify API responds on new `/goals` paths
7. For PR2: verify join table CRUD and relationship-scoped queries
8. For PR3: verify `?goal_id=X` filtering works on actions endpoints
9. For PR4: verify SSE events fire on join table operations; verify health endpoint returns correct metrics
10. For PR5: verify validate endpoints return correct valid/invalid splits
