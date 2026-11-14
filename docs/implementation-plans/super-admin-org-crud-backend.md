# Super-Admin Organization CRUD: Archive, Authz Gate, and Structured Errors

## Context

The frontend is building a `/admin` section for system SuperAdmins (`UserRole` with `role=SuperAdmin AND organization_id=NULL`) to manage organizations. Phase-1 FE CRUD (create / list-all / edit / delete-when-empty) is already built and live-verified on `jim/super-admin-org-crud`. The backend now owes the matching behavior, agreed on the coordinator board (decisions `org_archive_lifecycle`, `org_archive_phase1_confirmed`, `admin_org_crud_be_confirm`; answers on `admin_org_payload` / `admin_org_delete`).

Three problems this change addresses:

1. **No authorization on org mutations.** `POST/PUT/DELETE /organizations` are wrapped in `require_auth` only (`web/src/router.rs:522-538`) — any authenticated user can create/edit/delete any org today. A standing security gap; the SuperAdmin gate must land with the archive endpoints anyway (they are SuperAdmin-only).
2. **No safe, reversible "remove an org" path.** Deleting an org with coaching data raises a raw FK violation surfaced as an opaque 503. Product wants **Archive** (a reversible state, keeps all data) as the primary admin action, and `DELETE` reserved for genuinely-empty orgs with a clean 409.
3. **Unusable error surfaces.** Duplicate-name and non-empty-delete failures return raw 503 plain-text the admin form can't branch on. The FE needs discriminated 409s.

## Decisions locked with the user

- **Name uniqueness = DB unique index + app pre-check.** No `UNIQUE` constraint exists today (the entity's `#[sea_orm(unique)]` is cosmetic codegen; base schema has no index). **No dedup pre-flight needed** — prod currently holds 4 orgs, all distinct names/slugs (Shared Plate Strategies, BlockberryFin, Refactor Group, Pybites), confirmed against the live DB. Add `UNIQUE` indexes on `name` and `slug` plus an app-level pre-check in create/update for the clean `organization_name_taken` 409.
- **Record who archived.** Add `archived_by` (the authenticated user id) alongside `archived_at`. Both set on archive, both cleared on unarchive.
- **Write-freeze scope = relationships + sessions + member-adds.** Creating any of these three under an archived org returns `organization_archived` (409).
- **Archive in Phase 1.** Shallow org-level flag (no cascade-archive of existing children). `?status=` filter = `active|archived|all`, default `active`.

## Execution model — overseer + per-phase handoff

This build runs under the **overseer-handoff-workflow**: I (this agent) am the **persistent overseer** — I own this plan, write a self-contained gitignored handoff per phase, and **independently review** each finished phase (re-run gates, read the full diff, reproduce claims). Each phase is built by a **fresh implementer agent** with zero prior context that does **one phase, one commit, then STOPs and reports**. **Jim is the human gate**: he approves each phase before the next handoff. The implementer never reviews its own work or runs ahead.

Before Phase 1, copy this plan to `docs/implementation-plans/super-admin-org-crud-backend.md` as the committed living plan (per project convention); keep it current as decisions change. Acceptance criteria per phase are frozen in the handoff (the overseer owns the test assertions).

## Error discriminators (final wire, per board)

| `error` | HTTP | Where | `details` |
|---|---|---|---|
| `organization_not_empty` | 409 | DELETE when org has >=1 coaching_relationship | `{ coaching_relationship_count, coaching_session_count, member_count }` |
| `organization_name_taken` | 409 | create + rename name collision | `{ name }` |
| `organization_archived` | 409 | create relationship / session / member under archived org | none |

## Error chain — respecting layer boundaries

Verified against the real code. The repo has two precedents: the generic `EntityApiErrorKind::ValidationError { message, details }` -> `EntityErrorKind::Conflict { message, details }` -> web `error: "conflict"` (a SHARED discriminator), and the dedicated pass-through variants `CannotLinkCompletedGoal` / `GoalAlreadyLinkedToSession` (each its OWN hardcoded `error` string in `web/src/error.rs`).

Because the FE branches on distinct `error` strings, the generic `Conflict` path is unusable (it always emits `"conflict"`). Follow the **dedicated pass-through** precedent, one variant per discriminator, threaded through all three layers and keeping each layer's vocabulary:

- **`entity_api/src/error.rs`** — add to `EntityApiErrorKind` (this layer may be as rich as needed):
  - `OrganizationNotEmpty { coaching_relationship_count: u64, coaching_session_count: u64, member_count: u64 }`
  - `OrganizationNameTaken { name: String }`
  - `OrganizationArchived`
- **`domain/src/error.rs`** — add matching `EntityErrorKind` variants carrying only primitives (no `entity_api`/`serde` types leaked upward), and add explicit arms in `From<EntityApiError>` (`:94-139`). **Critical:** the match ends in `_ => EntityErrorKind::Other(...)` which renders 500 — the three new variants MUST get explicit arms or they silently become 500s.
- **`web/src/error.rs`** — add three arms in `handle_entity_error` (after `:162`), each `warn!` + `serde_json::json!` + `(StatusCode::CONFLICT, Json(body))`, mirroring `GoalAlreadyLinkedToSession` (`:152-162`); `organization_not_empty` builds the `details` object from the carried counts.
- **Name-collision backstop:** the app pre-check is primary; to convert a rare race-loss on the `UNIQUE` index, sniff the `DbErr` unique-violation in org `create`/`update` and map to `OrganizationNameTaken` rather than letting `From<DbErr>` flatten it to `SystemError` -> 503.

## Phased plan (one branch off `main`; one commit per phase)

### Phase 0 — Branch
Create the feature branch off `main`; copy this plan into `docs/implementation-plans/`.

### Phase 1 — Migrations + entity fields
- `migration/src/m<date>_add_archive_to_organizations.rs`: `ALTER TABLE refactor_platform.organizations ADD COLUMN archived_at TIMESTAMPTZ` (nullable) and `ADD COLUMN archived_by UUID` (nullable) + an FK `archived_by -> users(id) ON DELETE SET NULL` (deleting a user must not block; the marker just goes null). Mirror `m20260611_000000_add_topic_deleted_at.rs`. Plain column-adds need NO OWNER-TO step (that rule is for `create_type` only); the FK + indexes inherit table ownership.
- `migration/src/m<date>_add_organizations_name_slug_unique.rs`: `CREATE UNIQUE INDEX` on `name` and on `slug`. No dedup pre-flight (prod is clean).
- Register both in `migration/src/lib.rs` (a `mod` line + `Box::new(...)` each).
- `entity/src/organizations.rs`: add `archived_at: Option<DateTimeWithTimeZone>` and `archived_by: Option<Uuid>`, both `#[serde(skip_deserializing)]` (+ `#[schema(...)]` like `created_at`) so clients can't set them via plain POST/PUT — only the archive endpoints mutate them. Add the SeaORM `Relation`/`Related` wiring for the new `archived_by` FK to mirror existing relations.
- **Fix mock-test fallout in the same commit** (adding fields breaks literal `organizations::Model { .. }` constructions): `entity_api/src/organization.rs:130-145`, `web/src/extractors/organization_member_access.rs:165-175`, and the pinned SQL column-list assertion at `entity_api/src/organization.rs:178`. Grep for every `organizations::Model {` literal first.
- **Acceptance:** `cargo check` clean; migrations run up+down clean locally; existing mock suite green.

### Phase 2 — Error variants end to end
- Add the three variants across `entity_api/src/error.rs`, `domain/src/error.rs` (incl. explicit `From` arms), `web/src/error.rs`.
- **Tests:** web error-shape tests mirroring `web/src/error.rs:294-321` for all three discriminators incl. the `organization_not_empty` `details` counts.
- **Acceptance:** new error tests assert 409 + exact `error` string + details; suite green.

### Phase 3 — entity_api / domain org logic (transactional)
In `entity_api/src/organization.rs` (re-exported from `domain/src/organization.rs`). **Use a DB transaction wherever a read informs a conditional write** (pattern: `db.transaction(|txn| Box::pin(async move { ... }))`, the API-required form):
- `archive(db, id, archived_by)` / `unarchive(db, id)`: in a txn, read-then-conditional-write; idempotent no-op if already in target state (use `Unchanged(updated_at)` on no-op so the timestamp doesn't churn — precedent `update` `:40`). `archive` sets `archived_at=now, archived_by=Some(user)`; `unarchive` clears both.
- `delete_by_id` (`:46`): in a txn, count relationships (reuse `coaching_relationship::find_by_organization`, `entity_api/src/coaching_relationship.rs:188`), sessions, and members; if relationships >= 1 return `OrganizationNotEmpty { ...counts }` before the delete.
- `create` (`:14`) / `update` (`:32`): in a txn, pre-check name collision -> `OrganizationNameTaken { name }`, with the unique-violation `DbErr` sniff as backstop.
- `?status=` filter: change `find_by` (`:63`) from single-param `into_iter().next()` to explicit `user_id` + `status` key lookups (default `active`); thread `status` into `find_by_user` (`:86`) so the archived filter applies in BOTH the super-admin branch (`:98`) and the regular-user branch (`:102`). Only domain caller is `organization_controller.rs:48`.
- Re-export `archive`/`unarchive` from `domain/src/organization.rs`.
- **Tests:** mock tests mirroring `entity_api/src/goal.rs:434-452` for idempotent archive/unarchive (+ archived_by set/cleared), not-empty delete, name-taken, status filtering.

### Phase 4 — Write-freeze guards (3 chokepoints, in-transaction)
Load the org and reject with `OrganizationArchived` if `archived_at.is_some()`, inside the same transaction as the child insert so the check and write are atomic:
- **Relationships:** `entity_api/src/coaching_relationship.rs::create` (~`:30`).
- **Sessions:** `domain/src/coaching_session.rs::create` already loads the org at `:81-84` — add the check right after.
- **Members:** `entity_api/src/user.rs::create_by_organization` (`:44-70`) already opens a txn at `:49` — add the check inside it.
- **Tests:** one mock test per chokepoint asserting `OrganizationArchived` when the parent org is archived.

### Phase 5 — Web routes + authz extractor (advances issue #218) + controller
Per issue #218, use the **`FromRequestParts` extractor** pattern, NOT a new `Check`/`Predicate` middleware. This directly advances the issue's `UserIsAdmin -> AdminAccess extractor` checklist item.
- New `web/src/extractors/super_admin_access.rs`: `SuperAdminAccess { authenticated_user }` implementing `FromRequestParts`, modeled on `web/src/extractors/organization_member_access.rs` (the `FromRef<S>` template). Passes only when the caller has `SuperAdmin` role with `organization_id IS NULL`; else 403. (Named `SuperAdminAccess` for precision — org mutations are platform-super-admin-only, distinct from org-scoped `OrganizationMemberAccess`. Register in `web/src/extractors/mod.rs`.)
- Apply by adding `SuperAdminAccess { .. }` to the **handler signatures** of `create`, `update`, `delete`, and the new `archive`/`unarchive` (dual-use authorization + extraction in one step, the issue's preferred shape). GET `index`/`read` stay ungated. This avoids splitting the router and keeps `organization_routes` (`:522-538`) intact except for the two new routes.
- `web/src/controller/organization_controller.rs`: add `archive`/`unarchive` handlers (take `SuperAdminAccess` to get the acting user id for `archived_by`; return the updated org); update `index` to read+pass the `status` param; add the two `POST /:id/archive` + `/unarchive` routes; utoipa annotations (complete one-line summary, blank `///`, description).
- **Tests:** extractor unit tests mirroring `organization_member_access.rs` tests (200 for super-admin, 403 for org-admin/regular user); confirm GET stays open.

### Phase 6 — Live verify + gates (overseer reproduces independently)
Boot on real Postgres and exercise: archive/unarchive idempotency (+ `archived_by` recorded then cleared); `?status=active|archived|all` default-active exclusion; SuperAdmin gate (403 for non-super-admin on every mutation incl. archive; GET still 200); DELETE non-empty -> `organization_not_empty` 409 with correct counts, DELETE empty -> 200; duplicate name on create + rename -> `organization_name_taken` 409; create relationship / session / member under an archived org -> `organization_archived` 409. Then:
```
cargo test -p entity_api -p domain -p web --features "domain/mock,web/mock"
cargo clippy
cargo fmt
```
After live-verify, pin `AdminOrganizationCRUD` v2 on the board: all three discriminators, the `archived_at` + `archived_by` read fields, the `?status=` param, and the two new endpoints.

## Critical files
- `migration/src/` (two new migrations) + `migration/src/lib.rs`
- `entity/src/organizations.rs`
- `entity_api/src/organization.rs`, `entity_api/src/error.rs`, `entity_api/src/coaching_relationship.rs`, `entity_api/src/user.rs`
- `domain/src/organization.rs`, `domain/src/coaching_session.rs`, `domain/src/error.rs`
- `web/src/error.rs`, `web/src/extractors/super_admin_access.rs` (new) + `mod.rs`, `web/src/router.rs`, `web/src/controller/organization_controller.rs`

## Risks / conventions
- **`find_by` signature change** touches the hottest org read path; one domain caller, but verify.
- **Adding entity fields breaks pinned mock literals/SQL assertions** — fix all in Phase 1 (grep `organizations::Model {`).
- **Transactions are required** wherever a read gates a write (delete pre-check, name pre-check, archive read-modify-write, write-freeze guard + insert) to avoid TOCTOU races.
- Follow project conventions: new unit tests in separate `src/<mod>_tests.rs` (frozen-test discipline); terse comments; no imports inside fns; no `.unwrap()` in production; PG enum writes via ActiveModel `Set` (n/a — no new enums).
