# Coaching Session Title + Topics — Backend Master Implementation Plan

**Status:** Living document (overseer-owned). Kept current as decisions change.
**Method:** Overseer + per-phase implementer handoffs (see `.claude/skills/overseer-handoff-workflow`).
One persistent overseer plans + independently reviews; a fresh implementer builds each
phase from a self-contained handoff, commits once, and stops.

**Source of truth:** epic `refactor-group/refactor-platform-fe#412`.
**Backend issues:** `rs#346` (Title) · `rs#347` (Topics CRUD + reorder + authz) · `rs#348`
(relevance/immediacy rating). **Authz pattern:** `rs#218` (`FromRequestParts` extractors).
**FE counterparts:** `fe#413` (Title) · `fe#414` (Topics) · `fe#415` (rating).

---

## 1. What we're building

1. **Title** — one optional, human-authored `Option<String>` column on `coaching_sessions`.
   Not a new entity. Replaces "borrow the first linked goal's title" as the display name;
   the goal title remains a fallback (fallback chain lives in the FE).
2. **Topics** — a new `coaching_session_topics` table: 0..N rows per coaching session, each
   authored by a participant, with a text `body`, an author-controlled order, and (Phase 2)
   coachee-set `relevance` + `immediacy` ratings. Rows, **not** a JSON column, so concurrent
   edits don't clobber and delete stays author-scoped.

Topics are **not** goals and **not** the per-session `Agreement`. No change to either.

## 2. Frozen wire-contract invariants (do not violate in any phase)

These come straight from the epic and are the acceptance backbone for review:

- **`display_order` is backend-internal.** Never read, computed, or sent by the FE, and
  **never serialized**. Enforced with `#[serde(skip)]` on the entity field (SeaORM still
  reads/writes the column; serde never emits or accepts it).
- **The wire contract is array order, not the index.** Every read path
  (`GET .../topics` and the `Topics` include) returns topics **already sorted** by
  `display_order ASC, created_at ASC`. The FE never sorts client-side.
- **Reorder is a whole-list operation.** The FE sends the full ordered list of topic ids;
  the backend reassigns `display_order` from array position, in a transaction. A reorder
  whose id set ≠ the session's current topic id set is **rejected** (guards stale clients).
- **New topics append to the end.** `display_order = MAX(display_order for session) + 1`
  (or `0` if none). **Deletes may leave gaps** — harmless; the next reorder normalizes.
- **`updated_at` is touched by any mutation:** add / edit body / reorder / (Phase 2)
  rating change. Set explicitly in entity_api (this codebase has **no** DB `updated_at`
  trigger — every update sets it via `chrono::Utc::now()`).
- **Topic enums (Phase 2) are NOT NULL, default `Neutral`** (the untriaged state).

## 3. Architecture recap (grounding for every handoff)

Layered: `entity/` (SeaORM models) → `entity_api/` (CRUD) → `domain/` (re-export / business
logic) → `web/` (Axum handlers, extractors, routes). Error chain:
`entity_api::Error` → `domain::Error` → `web::Error` → HTTP.

**Closest existing analog = `notes`** (text body nested under a coaching session, per-row
`user_id` author, full CRUD). Model the whole stack on it, diverging only where the frozen
invariants require (ordering + reorder + include + author-scoped delete).

Key reference files (read these when writing/implementing a phase):

| Concern | Reference file | Notes |
|---|---|---|
| Entity (analog) | `entity/src/notes.rs` | Model + Relation + `skip_deserializing` pattern |
| Entity w/ `title` precedent | `entity/src/goals.rs` | already has `title: Option<String>` |
| Entity w/ PG enum | `entity/src/status.rs` | `DeriveActiveEnum`, `string_value`, `#[default]` |
| Coaching session entity | `entity/src/coaching_sessions.rs` | where `title` slots in; `has_many` relations |
| entity_api (analog CRUD) | `entity_api/src/note.rs` | `create`/`update`/`find_by_id`/`find_by`; `Set`/`Unchanged`; stamps `updated_at` |
| Enriched session + includes | `entity_api/src/coaching_session.rs` | `EnrichedSession`, `IncludeOptions`, `load_related_data`, `batch_load_*`, `assemble_enriched_session` |
| Include query param | `web/src/params/user/coaching_session.rs` | `IncludeParam` enum + comma-separated deserializer |
| Include controller wiring | `web/src/controller/user/coaching_session_controller.rs` | maps `IncludeParam` → `IncludeOptions` |
| Authz extractor template | `web/src/extractors/coaching_session_access.rs` | `FromRequestParts`, path id fallback, participant check |
| Extractor exports | `web/src/extractors/mod.rs` | `RejectionType = (StatusCode, String)` |
| Coach/coachee fields | `entity/src/coaching_relationships.rs` | `coach_id`, `coachee_id`, `includes_user()` |
| Nested route + handler | `web/src/controller/coaching_session/meeting_recording_controller.rs` + `web/src/router.rs` (~L749) | `CoachingSessionAccess` gating, `ApiResponse`, route wiring |
| Error → HTTP | `web/src/error.rs` | `NotFound`→404, `Unauthenticated`→401, `Invalid`→422, `Conflict`→409 |
| Success envelope | `web/src/controller/mod.rs` | `ApiResponse::new(status, data)` |
| Migration registry | `migration/src/lib.rs` | add `mod` + `Box::new(...)` in chronological order |
| Migration: new table + FK | `migration/src/m20251228_000001_add_actions_users_table.rs` | raw SQL, named FK, `ON DELETE`, `OWNER TO refactor` |
| Migration: nullable column | `migration/src/m20260511_000000_add_hydrated_at_to_coaching_sessions.rs` | `execute_unprepared` ALTER ADD COLUMN |
| Migration: PG enum + ownership | `m20260317_*_add_on_hold_to_status_enum.rs`, `m20260407_000002_add_transcriptions.rs` | `CREATE TYPE` + **`ALTER TYPE ... OWNER TO refactor`** |

## 4. Cross-cutting standards (enforced in every review)

- **Read `.claude/coding-standards.md` before implementing.** Imports at file top only —
  never inside fn bodies. Comments terse (one short line; no multi-paragraph). No em dashes.
  No Claude attribution in commits/PRs. Prefer functional/combinator Rust that reads like a
  sentence. No `.unwrap()` in production code (`?` / `match` / `let-else`).
- **No redundant type prefixes** — module path provides context (e.g. new extractor file is
  `coaching_session_topic_author_access.rs` exposing `CoachingSessionTopicAuthorAccess`,
  which is fine; but do not prefix the entity type redundantly).
- **PG enum writes** go through `ActiveModel` + `Set(enum)`, never `col_expr(Expr::value(enum))`
  (binds as text → Postgres 42804).
- **PG type ownership:** every `CREATE TYPE` is immediately followed by
  `ALTER TYPE refactor_platform.<name> OWNER TO refactor`. Same for new tables: `OWNER TO refactor`.
- **Frozen tests:** new unit tests live in a **separate** `src/<mod>_tests.rs` file wired via
  `#[cfg(test)] #[path = "..."] mod tests;` so the overseer can `chmod a-w` them. The
  **overseer owns the assertions** (specified in the handoff); the implementer transcribes
  them. Existing in-file `#[cfg(test)] mod tests` in `note.rs` is the older style — do **not**
  copy it for new test files.
- **Mock test invocation (exact):**
  `cargo test -p entity_api -p domain -p web --features "domain/mock,web/mock"`.
  Never `--workspace --features mock` (sea-orm/mock drops `DatabaseConnection: Clone`).
- **Adding a column ripples into mock SQL-shape assertions.** Mock tests that call
  `into_transaction_log()` hardcode the exact column list SeaORM emits (in entity field order).
  Adding a field to an entity breaks these at **runtime** (the test suite), not at compile time,
  so `cargo check` won't flag them. When a handoff adds a column, it must enumerate the affected
  expected-SQL strings, and the **overseer owns those expected strings** (e.g. the topics
  read-path `ORDER BY "...".display_order ASC, "...".created_at ASC`). Learned in P1
  (8 `coaching_sessions` SQL assertions needed the new `title` column inserted after
  `duration_minutes`).
- **Compile check:** `cargo check` (not `cargo build`). Lint/format: `cargo clippy`, `cargo fmt`.
- **Transactions:** `db.transaction(|txn| Box::pin(async move { ... }))` — the `Box::pin` is
  API-required, not optional.

## 5. Phase decomposition

Each phase ends in a **compilable, tested, single-purpose commit**. The overseer
independently re-runs gates, reads the full diff, and reproduces critical claims before
approving and writing the next handoff.

| # | Issue | Title | Layer(s) | Gate |
|---|---|---|---|---|
| **P1** | rs#346 | Title field end-to-end | migration + entity + entity_api + serialization | `title` round-trips; null when unset; on create/update/get/list/enriched |
| **P2** | rs#347 | Topics schema + entity | migration + entity + registrations | compiles; migration up/down clean; `OWNER TO refactor` |
| **P3** | rs#347 | Topics data layer | entity_api CRUD + append + reorder + domain + frozen mock tests | mock tests green; pre-sorted SQL; reorder reassigns + rejects mismatch |
| **P4** | rs#347 | Topics web layer | controller + routes + `CoachingSessionAccess` + author-only delete extractor + OpenAPI | CRUD + reorder reachable; non-author delete rejected; `display_order` never on wire |
| **P5** | rs#347 | Topics include | `IncludeParam`/`IncludeOptions`/`EnrichedSession` + `batch_load_topics` | `?include=topics` returns pre-sorted topics on enriched session |
| **P6** | rs#348 | Rating schema + entity | migration (enums NOT NULL default Neutral) + entity enum types + fields + serialization | new topics default `Neutral`; values persist + serialize |
| **P7** | rs#348 | Rating endpoint + coachee authz | entity_api rating write + domain + controller + coachee-only extractor + OpenAPI | coachee can rate; non-coachee rejected; touches `updated_at` |

**Phase grouping vs the epic:** P1–P5 = epic **Phase 1** (rs#346 + rs#347). P6–P7 = epic
**Phase 2** (rs#348), which the epic permits splitting to its own milestone — confirm scope
before starting P6.

**Reasonable merges** (if fewer, larger phases are preferred): P2+P3 (whole data layer in one
commit) and P6+P7 (whole rating feature in one commit). The plan recommends the granular split
because migration correctness (FK `ON DELETE`, ownership, indexes) is a distinct review concern
from CRUD/reorder logic, and a focused migration commit is trivial to review and revert.

### Build progress

- **P1 — Title — ✅ APPROVED** (overseer-reviewed 2026-06-07). Branch
  `feat/346-coaching-session-title`, commit `ec01c9e`. All four gates reproduced independently
  by the overseer: fmt clean, `cargo check` clean, mock suite **173/184/88** + integration bins
  (0 failed), clippy clean (both default and mock configs). Clear-to-null works via the
  double-option; 8 frozen-test assertions in `mod_tests.rs` (a-w) cover absent/null/value at
  both deserialize and map-build layers. Implementer-flagged divergence (8 mock SQL-shape
  assertions updated) verified legitimate and behavior-preserving. **Pre-merge:** run the
  migration up/down against a live PG (deferred per handoff; trivial nullable ADD/DROP, runs in
  CI/preview migrator).
- **P1b — empty/whitespace title → NULL normalization — ✅ APPROVED** (overseer-reviewed
  2026-06-07). Same branch, commit `b2fc22a`. BE-layer invariant (not web DTO): `normalize_title`
  in `entity_api::create` + `normalize_title_in_update_map` called in `domain::update` (mirrors
  the `validate_duration_in_update_map` pattern). Trims; empty → `None`; explicit-null (clear) and
  omitted (no-op) preserved. Gates reproduced: mock suite **191/173/88**, 7 new frozen tests in
  `coaching_session_normalize_tests.rs` (a-w). Result: BE never stores/returns `""` for title.
  Added because the FE wire-contract question (`coaching_session_title_wire_contract`) hinged on
  the empty-string semantics.
- **P2 — Topics schema + entity — ✅ APPROVED** (overseer-reviewed 2026-06-07). Branch
  `feat/347-coaching-session-topics` (off main; P2–P5 stack here → one PR for rs#347), commit
  `b1491d8`. Migration `m20260607_000001_create_coaching_session_topics`: both FKs
  `ON DELETE CASCADE ON UPDATE CASCADE`, `(coaching_session_id, display_order)` index,
  `OWNER TO refactor`, table-dropping `down`. Entity `coaching_session_topics`: `body: String`
  (NOT NULL), `display_order: i32` `#[serde(skip)]`, `has_many` + `Related` on `coaching_sessions`.
  Gates reproduced: check/clippy/fmt clean, mock suite **173/184/80** (baseline, no new tests this
  phase — entity+migration have no unit-testable logic). **Pre-merge:** run migration up/down on
  live PG. **Carried to P3:** add a serialization test asserting a topic Model omits `display_order`
  (no serialization path existed in P2 to exercise the frozen wire invariant).
- **P3 — Topics data layer — ✅ APPROVED** (overseer-reviewed 2026-06-07). Branch
  `feat/347-coaching-session-topics`, commit `79b549a`. `entity_api/src/coaching_session_topic.rs`:
  CRUD + append (`next_display_order` = max+1) + pre-sorted `find_by_coaching_session_id`
  (`ORDER BY display_order, created_at`) + **non-transactional** `reorder` (validate permutation →
  reassign by index → return sorted). New `EntityApiErrorKind::TopicReorderMismatch` → domain
  `From` arm → `DomainErrorKind::Validation` (422). Domain re-export mirrors `note`. 10 frozen tests
  in `coaching_session_topic_tests.rs` (a-w): append teeth, reject-mismatch teeth, serialization
  (no `display_order`), ordering SQL-shape, reorder-mismatch behavioral. Gates reproduced:
  check/clippy/fmt clean, mock suite **entity_api 194 / domain 173 / web 80** (report swapped the
  entity_api/domain labels — corrected here). **Reorder guard mutation-tested:** defeating the guard
  makes the mismatch test FAIL, restoring it passes — real teeth. Non-transactional by design (no
  transactions exist in the codebase; epic tolerates last-write-wins).
- **P4 — Topics web layer + authz — ✅ APPROVED** (overseer-reviewed 2026-06-07). Branch
  `feat/347-coaching-session-topics`, commit `0e36973` (first attempt dropped on an infra socket
  error leaving nothing durable; re-run fresh from clean P3 baseline). Routes
  `GET/POST /topics`, `PATCH /topics/reorder`, `PUT/DELETE /topics/:topic_id` under `require_auth`.
  **Authz by extractor composition:** `CoachingSessionTopicAccess` (composes `CoachingSessionAccess`
  + topic-belongs-to-session) for update; `CoachingSessionTopicAuthorAccess` (+ author) for delete;
  index/create/reorder use `CoachingSessionAccess`. All failures fail-closed to **404**. reorder
  not special-cased → 422 propagates. 3 full HTTP integration authz tests (login→cookie→DELETE):
  author→200, non-author→404, wrong-session→404. Gates reproduced: clippy/fmt clean, web **83**
  (+3). **Both authz guards MUTATION-TESTED:** defeating the session-match guard fails the
  wrong-session test; defeating the author guard fails the non-author test — both real teeth.
  Scope: 5 web files, no data-layer/entity/migration/plan-doc touched. OpenAPI registers 5 handlers
  + 3 DTOs + topic Model.
- **P5 — Topics include — ✅ APPROVED** (overseer-reviewed 2026-06-07). Branch
  `feat/347-coaching-session-topics`, commit `3d3f124`. `IncludeParam::Topics` (wire `"topics"`) →
  `IncludeOptions.topics` → `EnrichedSession.topics` (`skip_serializing_if`); `batch_load_topics`
  one-query, grouped-per-session, pre-sorted `display_order, created_at`; wired into
  `load_related_data` + `assemble_enriched_session`. **Implementer caught an interaction bug**: the
  `find_by_user_with_includes` early-return short-circuit didn't know about topics, so a
  topics-only request would have skipped loading — fixed by adding `&& !options.includes.topics` to
  the guard. Gates reproduced: 173/194/83 (no new tests — mirrors untested sibling loaders). 4
  `IncludeOptions` test literals got the new field. Scope: 3 files, no out-of-scope.
- **✅ Epic Phase 1 (P1–P5) COMPLETE** — Title (rs#346, PR #349) + Topics CRUD/reorder/authz/include
  (rs#347, PR #350).
- **P6 — Rating schema + enums — ✅ APPROVED** (overseer-reviewed 2026-06-07). Branch
  `feat/348-topic-rating` (off feat/347; stacked), commit `c30a9bd`. Migration
  `m20260607_000002_add_topic_rating_enums`: `CREATE TYPE topic_relevance`/`topic_immediacy` (each
  `+ OWNER TO refactor`), columns `NOT NULL DEFAULT 'neutral'`, `down` drops cols then types. Entity
  enums `Relevance` (neutral/peripheral/worth_exploring/central) + `Immediacy`
  (neutral/can_wait/soon/pressing), serde = PascalCase variant on the wire (like `status`). `update`
  preserves rating via `Unchanged`; `create`/`reorder` unchanged (DB default applies). Frozen test
  re-frozen: asserts relevance+immediacy serialized, display_order not, ORDER BY unchanged; SQL now
  has `CAST(... AS "text")` enum cols. Enums re-exported `entity_api`→`domain` (VERIFIED correct:
  web has no `entity` dep; `domain::<enum>::Type` is the established pattern, mirrors provider/status;
  utoipa `body = entity::...` annotations are macro schema-name refs, not a real entity dep). Gates
  173/194/83. **Pre-merge:** run migration up/down on live PG (PG enum + OWNER TO).
- **P7 — Rating endpoint + coachee authz — ✅ APPROVED** (overseer-reviewed 2026-06-07). Branch
  `feat/348-topic-rating`, commit `d57cc03`. `entity_api::set_rating` (via `topic.into()` + `Set`,
  stamps `updated_at`); domain re-export; route `PATCH /coaching_sessions/:id/topics/:topic_id/rating`;
  new `CoachingSessionTopicCoacheeAccess` extractor — **coachee-only (403 for a coach), topic-in-session
  (404)**. 2 HTTP integration tests (coachee→200, coach→403). Gates reproduced: clippy/fmt clean, web
  **85** (+2). **Coachee guard MUTATION-TESTED:** defeating it fails the 403 test — real teeth. Scope:
  5 files, no migration/enum/entity change. **Pre-merge:** live-PG migration check (P6 enums).
- **✅✅ ENTIRE BUILD COMPLETE (P1–P7).** Title (rs#346, PR #349) · Topics CRUD/reorder/authz/include
  (rs#347, PR #350) · rating (rs#348, PR pending). Every phase independently reviewed; reorder guard,
  both topic authz guards, and the coachee guard all mutation-tested.

---

## 6. Per-phase detail

### P1 — Title field (rs#346)

**Migration:** new file `migration/src/mYYYYMMDD_000000_add_title_to_coaching_sessions.rs`;
register in `migration/src/lib.rs` (mod + `Box::new`). `up`: `ALTER TABLE
refactor_platform.coaching_sessions ADD COLUMN title VARCHAR` (nullable, no default).
`down`: `DROP COLUMN title`. (Pattern: `m20260511_*_add_hydrated_at`.)

**Entity:** add `pub title: Option<String>,` to `entity/src/coaching_sessions.rs` `Model`
(a *deserializable* field — no `skip_deserializing`, so it's accepted on writes), placed near
`date`/`meeting_url`.

**entity_api:** wire `title` through `coaching_session` create + update so it's `Set` on
create and updatable (the create/update build `ActiveModel`s field-by-field; a new field is
**not** automatically threaded). Implementer must locate and update those functions; if create
goes through a different path (e.g. a dedicated params struct), flag it.

**Serialization:** `title` appears automatically in get / list / enriched payloads because
`EnrichedSession` `#[serde(flatten)]`s the `Model`. Confirm, don't assume.

**Acceptance:** `title` round-trips (set → read back identical); `null`/`None` when unset;
present in single-get, list, and enriched responses; accepted on create + update; **clearable
to null on update** (`"title": null` → `None`) while omission leaves it unchanged (see §8).
**Gate:** `cargo check`; mock suite green; clippy + fmt clean.

### P2 — Topics schema + entity (rs#347)

**Migration:** new `coaching_session_topics` table (raw SQL, pattern = actions_users):
`id UUID PK DEFAULT gen_random_uuid()`, `coaching_session_id UUID NOT NULL`,
`user_id UUID NOT NULL`, `body TEXT NOT NULL`, `display_order INT NOT NULL`,
`created_at TIMESTAMPTZ NOT NULL DEFAULT now()`, `updated_at TIMESTAMPTZ NOT NULL DEFAULT now()`.
FKs: `coaching_session_id → coaching_sessions(id) ON DELETE CASCADE`,
`user_id → users(id) ON DELETE CASCADE` (**both CASCADE — decided**; topics are transient
session artifacts, and `user::delete` does a bare `delete_by_id` with no cleanup so a
non-cascading author FK would break user deletion). Index on `(coaching_session_id, display_order)`.
**`ALTER TABLE ... OWNER TO refactor`.** `down`: `DROP TABLE`. Register in `lib.rs`.

**Entity:** new `entity/src/coaching_session_topics.rs` modeled on `notes.rs`:
`id` (`skip_deserializing`, pk), `coaching_session_id`, `body: String`, `user_id`
(`skip_deserializing`), `display_order: i32` **`#[serde(skip)]`**, `created_at`/`updated_at`
(`skip_deserializing`). `belongs_to` relations → `coaching_sessions` + `users`. Register
`pub mod coaching_session_topics;` in `entity/src/lib.rs` and add
`#[sea_orm(has_many = "super::coaching_session_topics::Entity")]` + `Related` impl on
`coaching_sessions`.

**Gate:** `cargo check` whole workspace; migration `up` then `down` run clean against a real
PG (overseer verifies up/down idempotency + ownership).

### P3 — Topics data layer (rs#347)  *(correctness-critical)*

**entity_api** `entity_api/src/coaching_session_topic.rs` + register in `entity_api/src/lib.rs`:
- `create(db, model, user_id)` — append: `display_order = MAX(display_order WHERE
  coaching_session_id = ..) + 1`, else `0`; stamp both timestamps.
- `find_by_id(db, id)` — `Option<Model>` / `RecordNotFound` like `note.rs`.
- `update(db, id, model)` — set `body`, stamp `updated_at`, keep id/session/user/created_at
  `Unchanged`.
- `delete(db, id)`.
- `find_by_coaching_session_id(db, id)` — **`ORDER BY display_order ASC, created_at ASC`**.
- `reorder(db, coaching_session_id, ordered_ids: Vec<Id>)` — in a transaction: load current
  topic ids for the session; if `set(ordered_ids) != set(current)` return a validation error;
  else reassign `display_order = index` for each id and stamp `updated_at`.

**domain** `domain/src/coaching_session_topic.rs` — re-export entity_api fns (mirror
`domain/src/note.rs`), wrapping only if business logic is needed.

**Frozen mock tests** (overseer owns assertions) in `entity_api/src/coaching_session_topic_tests.rs`,
wired via `#[cfg(test)] #[path = "coaching_session_topic_tests.rs"] mod tests;`:
- `find_by_coaching_session_id` emits SQL containing
  `ORDER BY "coaching_session_topics"."display_order" ASC, "coaching_session_topics"."created_at" ASC`.
- `create` issues a MAX(display_order) lookup then an insert (append semantics).
- `reorder` with a matching id set reassigns order by array index and stamps `updated_at`.
- `reorder` with a **mismatched** id set returns the validation error and writes nothing
  (teeth: the test must fail if the guard is removed).
- **(carried from P2)** a serialized topic `Model` (via `serde_json`) **omits `display_order`**
  and includes `id`/`coaching_session_id`/`user_id`/`body`/`created_at`/`updated_at` — gives the
  frozen "`display_order` never on the wire" invariant real teeth now that a Model is in hand.

**Gate:** `cargo test -p entity_api -p domain --features "domain/mock"` green; clippy + fmt.
Overseer confirms the mismatch test actually fails when the set-equality guard is deleted.

### P4 — Topics web layer (rs#347)

**Routes** in `web/src/router.rs` (pattern = `coaching_session_meeting_recording_routes`),
all under `from_fn(require_auth)` + `CoachingSessionAccess` gating:
- `GET  /coaching_sessions/:coaching_session_id/topics` → index (pre-sorted)
- `POST /coaching_sessions/:coaching_session_id/topics` → create (author = authed user)
- `PUT  /coaching_sessions/:coaching_session_id/topics/:topic_id` → update body
- `PATCH /coaching_sessions/:coaching_session_id/topics/reorder` → reorder (full id list)
- `DELETE /coaching_sessions/:coaching_session_id/topics/:topic_id` → delete (**author-only**)

**Author-only delete extractor** `web/src/extractors/coaching_session_topic_author_access.rs`
(template = `coaching_session_access.rs`): load the topic by `:topic_id`, assert it belongs to
`:coaching_session_id`, assert `topic.user_id == authenticated_user.id`, else reject. Export in
`web/src/extractors/mod.rs`. (Alternative per rs#347: `CoachingSessionAccess` + a domain
ownership assertion — plan **recommends the extractor** per rs#218.) Decide the non-author
status code in review: `403 Forbidden` vs `404 Not Found` (404 avoids disclosing existence).

**Controller** `web/src/controller/coaching_session/topic_controller.rs`: handlers take
`CoachingSessionAccess(session)` (and `CoachingSessionTopicAuthorAccess` for delete),
`State<AppState>`, `Json<...>`; return `ApiResponse::new(...)`. **Reorder rejection** maps to
`422 Unprocessable Entity` (validation/precondition; `409 Conflict` is the alternative — pick
422 unless review prefers conflict). `#[utoipa::path]` on every handler; register handler fns
+ schemas in the `ApiDoc` `#[openapi(...)]` list in `router.rs`.

**Acceptance:** full CRUD + reorder reachable and authorized; non-author delete rejected;
reorder with mismatched id set → 422; `display_order` absent from every response body.
**Gate:** `cargo check`; `cargo test ... --features "domain/mock,web/mock"`; clippy + fmt.
Overseer reproduces non-author-delete rejection and `display_order` absence independently.

### P5 — Topics include (rs#347)

- `web/src/params/user/coaching_session.rs`: add `Topics` to `IncludeParam`.
- `entity_api/src/coaching_session.rs`: add `topics: bool` to `IncludeOptions` (+ `none()`),
  `topics: Option<Vec<coaching_session_topics::Model>>` to `EnrichedSession`
  (`skip_serializing_if = "Option::is_none"`), `batch_load_topics(db, &session_ids)`
  (**pre-sorted**, grouped into `HashMap<Id, Vec<Model>>`), call it in `load_related_data`
  when `includes.topics`, populate in `assemble_enriched_session`.
- `web/src/controller/user/coaching_session_controller.rs`: map
  `params.include.contains(&IncludeParam::Topics)` → `IncludeOptions { topics, .. }`; update
  the `include` param doc string in `#[utoipa::path]`.

**Acceptance:** `?include=topics` returns each session's topics pre-sorted; omitted otherwise.
**Gate:** mock suite green; clippy + fmt.

### P6 — Rating schema + entity (rs#348)  *(epic Phase 2 — confirm scope first)*

**Migration:** `CREATE TYPE refactor_platform.topic_relevance AS ENUM ('neutral',
'peripheral', 'worth_exploring', 'central')` and `topic_immediacy AS ENUM ('neutral',
'can_wait', 'soon', 'pressing')` — **each followed by `ALTER TYPE ... OWNER TO refactor`**.
Add columns `relevance` + `immediacy` to `coaching_session_topics`, **NOT NULL DEFAULT
'neutral'**. (`relevance` low-end = `peripheral`, decided; remaining display labels can be
refined FE-side without a migration.) `down` reverses.

**Entity:** new `entity/src/topic_relevance.rs` + `entity/src/topic_immediacy.rs`
(`DeriveActiveEnum`, `rs_type="String"`, `db_type="Enum"`, `enum_name=...`, `#[default]
Neutral`, modeled on `entity/src/status.rs`). Add `pub relevance: Relevance` + `pub immediacy:
Immediacy` to the topic `Model` (deserializable on the rating write, or gate via endpoint —
decide in P7). Register modules in `entity/src/lib.rs`.

**Acceptance:** new topics default `Neutral` on both axes; values persist and serialize on reads.
**Gate:** `cargo check`; migration up/down + ownership; mock suite green.

### P7 — Rating endpoint + coachee authz (rs#348)

**entity_api/domain:** `set_rating(db, topic_id, relevance?, immediacy?)` (or two fns); write
via `ActiveModel` + `Set(enum)` (never `Expr::value`); stamp `updated_at`.

**Web:** `PATCH /coaching_sessions/:coaching_session_id/topics/:topic_id/rating` (or
relevance/immediacy sub-paths). Gate to the **coachee** of the relationship — new
`CoachingSessionCoacheeAccess` extractor that reuses the session+relationship load and asserts
`coaching_relationship.coachee_id == authenticated_user.id`. Applies to **all** topics
regardless of author. `#[utoipa::path]` + `ApiDoc` registration.

**Acceptance:** coachee sets/changes both axes (persist + returned on reads); non-coachee
rating write rejected; touches `updated_at`.
**Gate:** `cargo check`; full mock suite; clippy + fmt. Overseer reproduces non-coachee
rejection on an isolated setup.

---

## 7. Decisions (resolved with the user 2026-06-07)

1. **Title editability** — **either party**, last-write-wins. ✅
2. **Rating (rs#348) scope** — **in scope now** as P6–P7. ✅
3. **Relevance enum** — `Neutral / Peripheral / WorthExploring / Central` (low-end =
   `peripheral`). `immediacy` — `Neutral / CanWait / Soon / Pressing`. Display labels may be
   refined FE-side later without a migration; DB `string_value`s are stable. ✅
4. **Reorder mismatch status code** — **422 Unprocessable Entity**. ✅
5. **Non-author delete status code** — **404 Not Found** (hides topic existence). ✅
6. **`ON DELETE` for both topic FKs** — **CASCADE** (session FK *and* author/`user_id` FK). ✅

## 8. Follow-ups / known gaps (carry forward)

- **Title clear-to-null IS supported in P1** via a double option on the *update* DTO only:
  `UpdateParams.title: Option<Option<String>>` with `#[serde(default, deserialize_with = ...)]`
  (absent → `None` = unchanged; `null` → `Some(None)` = clear; value → `Some(Some(v))` = set).
  `into_update_map` emits `Value::String(None)` for the clear case, which `mutate::update`
  already turns into `SET title = NULL` with no data-layer change. `CreateParams` stays plain
  `Option<String>`. (The older `meeting_url` field still lacks clear semantics; migrate it to
  the same pattern later if the FE needs it.)

- **Concurrent reorders are last-write-wins** (epic-accepted). Optimistic concurrency is a
  later add if needed — not in scope.
- **Title fallback chain lives in the FE** (`coachingSessionTitle()`); backend only stores the
  optional column. No backend fallback logic.
- **Phase 3/4 epic backlog** (2×2 priority matrix display, LLM auto-title) are FE/derived
  layers (`fe#416`, `fe#417`) — **no backend work** beyond the fields P6 adds.
- **Early-return interaction has no regression test** (P5). `find_by_user_with_includes`
  short-circuits when no includes are requested; the guard now lists `topics`, but neither topics
  nor the sibling `goal`/`agreements` includes have a test proving a single-include request
  actually loads (skips the early return). Verify a topics-only `?include=topics` loads during
  end-to-end testing; consider adding a guard test.
- **Cross-repo wire agreement** is part of epic DoD: Title `Option<string>`; topics returned
  pre-sorted; non-null topic enums with `Neutral` default; no FE tolerance hacks. Coordinate
  via the shared blackboard at each phase boundary that changes the wire.

## 9. v4 Redesign — priority + status lifecycle (decided 2026-06-09)

Supersedes the relevance/immediacy 2×2 rating (P6/P7). Driven by FE board proposal
`topic_priority_status_redesign`. **Nothing shipped** (PRs draft), so we modify in place rather
than ship-then-revert. The epic's Phase-3 2×2 priority matrix (#416) is **dropped**.

**Decisions:**
- **Change A — single `priority`** (`topic_priority` ∈ `Low | Medium | High`, **nullable/Option**,
  unset by default — cleaner than the always-`Neutral` two-enum shape). Drops
  `relevance`/`immediacy` entirely. Rating endpoint stays coachee-only, body `{ priority? }`.
  Create accepts optional `priority` (restore fidelity).
- **Change B — `status` lifecycle** (`topic_status` ∈ `Open | Discussed | Deferred`, `NOT NULL
  DEFAULT 'open'`). **Authz: either participant** (reuses `CoachingSessionTopicAccess`); rating
  stays coachee-only (`CoachingSessionTopicCoacheeAccess`). New `PATCH .../status` endpoint.
- **Change C — carry-over on defer = Q1 option (b)**: a `Deferred` topic copies forward into the
  **next session at session-create/hydration time** (mirrors goals' `link_in_progress_goals`),
  setting `carried_from_topic_id` (nullable self-FK). Source stays `Deferred` in its session.
  Handles "no next session yet" for free (waits); robust to rescheduling.
- **`CoachingSessionHydrationTask` registry** (full, loosely-coupled): a trait + registry +
  runner that de-dups the inline task sequence in `create`/`ensure_hydrated`. Context
  `CoachingSessionHydrationContext { txn, db, config, &mut session, relationship }`. Existing work
  becomes `MeetingUrlHandler` + `GoalsCarryForwardHandler`; new `TopicsCarryOverHandler`. **Tiptap
  doc stays the external-resource bracket** (needs delete-on-failure compensation; don't fold it
  into the trait). Named for `CoachingSession` (not `Session` — user/auth sessions exist) and
  `Hydration` (consistent with `ensure_hydrated`/`hydrated_at`/`mark_hydrated`, the `hydrated_at`
  flag is the single source of truth for "has this run").
- **Version `CoachingSessionTopics` v4** (breaking). SSE `topics_changed` unchanged (still fires
  on every mutation incl. the new status writes + carry-over).

**Branch:** `feat/topic-priority-lifecycle` off `feat/topic-sse-events` (keeps table/CRUD/reorder/
authz/include/SSE; the redesign commits replace rating + add status/carry-over/hydration). Will
**supersede #351 (rating) + #352 (SSE)** into one PR (base `feat/347` = #350); close those when the
replacement PR is up. Dev DB already rolled back past the topics+rating migrations (2026-06-09).

**Phases (executed shape — R1 merged schema+data-layer+web-DTOs into one compiling vertical):**
- **R1 — Data-model swap [DONE, commit `31fe0b6`].** Migration `m20260607_000002_add_topic_priority_status`:
  `topic_priority` (nullable enum low/medium/high) + `topic_status` (NOT NULL default `open`) +
  `carried_from_topic_id` (nullable self-FK `ON DELETE SET NULL`), both types `OWNER TO refactor`.
  Replaced `topic_relevance`/`topic_immediacy` entity enums with `topic_priority`/`topic_status`;
  Model `priority: Option<Priority>`, `status: Status`, `carried_from_topic_id: Option<Id>`
  (`#[serde(skip_deserializing)]`). Data layer: `create(priority?)`, `set_rating`→`set_priority`,
  new `set_status`. Domain wrappers + web `CreateParams`/`RatingParams` → `{ priority? }`; rating
  endpoint sets priority. Frozen tests updated + re-frozen. **Overseer-verified:** all gates green
  (mock 176/194/85, fmt, clippy); migration up/down + `ON DELETE SET NULL` + `status` default
  proven against real Postgres (scratch schema). NOT in R1: status endpoint, carry-over logic.
- **R2 — `CoachingSessionHydrationTask` registry (HIGH BLAST RADIUS) [DONE, commit `b52922a`].**
  New `domain/src/coaching_session_hydration.rs`: trait + `CoachingSessionHydrationContext { txn,
  session, relationship }` + registry + runner + `GoalsCarryForwardTask` (combinator form).
  `create`/`ensure_hydrated` route the goal tail through the runner, publish via generic
  `publish_events`; `publish_goals_linked` deleted. **Pure behavior-preserving refactor**:
  overseer-verified zero test changes (test module byte-identical), exact mock query sequences +
  events unchanged (domain 176/0), non-mock build + live boot HTTP 200. Reserved: full goals
  carry-forward e2e (disproportionate given exact-sequence mock coverage). Context deliberately
  minimal (`db`/`config`/`organization` added per-task later).
- **R2b — `TopicsCarryOverTask` [DONE, commit `46939ad`].** Policy = **Deferred-only** (user chose
  Option B, not Open+Deferred; board decision `topics_carry_over_policy` corrected). `entity_api`:
  `coaching_session::find_prior_session` (relationship + `date < before`, desc, one);
  `coaching_session_topic::carry_over` (filters `Deferred` **in Rust** to dodge the enum-in-WHERE
  42804 trap; copy resets status→Open, preserves body/priority/user_id, appends order, stamps
  `carried_from_topic_id`); `find_by_coaching_session_id` widened to `&impl ConnectionTrait`.
  `TopicsCarryOverTask` registered 2nd; **context UNCHANGED, `create`/`ensure_hydrated` bodies
  UNCHANGED** (seam paid off). Emits `topics_changed` only when ≥1 carried. Overseer-verified:
  gates green (entity_api 198 / domain 177), Deferred-only filter **mutation-tested (test fails when
  guard defeated)**, frozen test re-frozen. Real-PG write primitives (enum INSERT via ActiveModel+Set,
  self-FK) already proven in R1; fresh full carry-over e2e reserved as disproportionate.
- **R3 — Status endpoint + OpenAPI [DONE, commit `23c7a78`].** `PATCH .../topics/{id}/status`
  (either-participant via `CoachingSessionTopicAccess` — no 403; required `StatusParams { status }`)
  calls `TopicApi::set_status` (publishes `topics_changed`); route + OpenAPI `paths`/`schemas` wired.
  Overseer-verified: gates green (web 85), and **live** — booted the binary, new route returns 401
  (auth chain runs ⇒ wired) vs an unregistered subpath that never reaches auth; served OpenAPI
  (`/api-docs/openapi2.json`) advertises the path + `StatusParams`. No web handler test (sibling
  `set_rating` has none; authz covered by extractor tests, behavior by domain `set_status` test).

**Redesign feature-complete (R1–R3 + R5).** Remaining: refresh the integration branch with the
redesign; open the replacement PR (base `feat/347` = #350) and **supersede/close #351 (rating) +
#352 (SSE)**; coordinate the breaking wire change with the frontend (contract v4 + Deferred-only
carry-over decision are on the board).
- **R5 — Contract v4 + board [DONE].** Posted `CoachingSessionTopics` v4 + answered the proposal's
  3 asks (Q1→b, status authz→either-participant, version→v4).
