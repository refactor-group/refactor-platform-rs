# Search Capability (Backend) — Phased Implementation Plan

**Status:** Proposed
**Date:** 2026-08-25
**Author:** Raymond Nambaale & Claude

## Context

The platform has no search capability of any kind today. There is no text search in `entity_api/` (no `ILIKE`, no `tsquery` anywhere), every index is a plain btree (the one exception is the functional `LOWER(email)` index on `users`), and no Postgres extensions are installed (`pg_trgm` and `pgvector` are both absent).

Users need to find things: "the session where we discussed the quarterly review", "goals mentioning public speaking", "who said X in a transcript". Two kinds of consumers need this:

- **Humans** — coaches and coachees in the web UI (a global search box), organization admins, and super admins.
- **AI agents** — MCP tools per [the MCP server architecture](../architecture/mcp_server.md), which already defers a "dedicated action search tool" and specifies a `keyword` filter vocabulary.

This plan defines the search API contract, the authorization model, and a phased delivery path from a simple-but-useful keyword search to full semantic and hybrid search.

### Requirements

**Searchable entity types**

| Type | Text searched | Who can search it |
|---|---|---|
| Coaching sessions | `title` | Participants; org admins (their orgs); super admins |
| Notes | projected plain text, from PR 5 (see [Notes](#notes-tiptap-content)) | Same |
| Transcripts | `transcript_segments.text` | Same |
| Goals | `title`, `body` | Same |
| Actions | `body` | Same |
| Agreements | `body` | Same |
| Topics | `body` (excluding soft-deleted) | Same |
| Members (users) | name fields, email | Org admins (their orgs) and super admins only |
| Organizations | `name`, `slug` (active only) | Super admins only |

**Visibility tiers** — search must never return anything the caller cannot already see:

1. **Regular user**: only entities reachable through coaching relationships they participate in *and* whose organization they are currently a member of — the exact rule in `coaching_relationships::Model::grants_access_to` (`entity/src/coaching_relationships.rs`). A user removed from an org loses access to history they took part in.
2. **Organization admin**: tier 1 plus every relationship in the org(s) where they hold the `Admin` role.
3. **Super admin** (global role, `organization_id IS NULL`): everything.

**Filters**: `created_at`/`updated_at` date ranges (timezone-aware), by user, by organization, by coaching session, by status, by entity type, and by goal linkage for actions (linked to a particular goal, linked to any goal, or linked to none).

### Relationship to Tiptap's search API

The product question "can we copy or emulate Tiptap's search API?" resolves to: **emulate the shape, do not use the product.**

- Tiptap Cloud's Semantic Search is an add-on that only indexes documents stored in Tiptap Cloud. It structurally cannot see our Postgres-resident content (transcripts, goals, actions, agreements, topics), which is most of the corpus.
- Its documentation page has been removed (404), and its request body is documented inconsistently across sources — it is not a stable dependency.
- Its *spirit* is right and we keep it: a single search call, `{query, limit}` in, a flat scored result array out. We extend that with the type discriminator, authorization scoping, and navigation context our consumers need.

This also stays correct as the platform moves from Tiptap Cloud to the planned self-hosted `docs-collab-server` workspace member (see [Notes](#notes-tiptap-content)).

## Decisions

- **Keyword search first, semantic later, both first-class.** Phase 1 ships a fast, deterministic, always-available keyword search on Postgres full-text search — no extensions, no embedding provider, no per-query cost. Semantic (`mode=semantic`) and hybrid (`mode=hybrid`) are committed later phases, not maybes: the API reserves the `mode` parameter with all three values from day one, defaulting to `keyword`, and the response shape is identical in every mode so adding modes is additive, never breaking. Pagination *depth* is the one deliberately mode-specific contract point — semantic and hybrid paginate a bounded pool rather than the full corpus; see [Semantic and hybrid phases](#semantic-and-hybrid-phases).
- **`websearch_to_tsquery('english', q)`** rather than `plainto_tsquery`: it never errors on malformed user input and gives humans and LLMs `"quoted phrases"`, `-negation`, and `or` for free. `ILIKE '%q%'` is rejected for content search (cannot be indexed without `pg_trgm`, no ranking, no multi-word semantics). `pg_trgm` fuzzy matching is deferred as an optional add-on.
- **Exception — users and organizations** are searched with `ILIKE prefix%` on name/email fields plus the existing `LOWER(email)` index: the tables are small, and stemming hurts proper names. No new indexes needed.
- **GIN expression indexes**, not stored generated `tsvector` columns, for phase 1: no schema change, no entity regeneration, no write amplification. The tradeoff is that the query must repeat the index expression exactly, so the expression SQL text lives once as constants in `entity_api/src/search/` shared by index DDL documentation and query code. Revisit stored columns with `setweight` (title vs body weighting) only if ranking tuning demands it. These indexes are also deliberately dispensable — cheap to build, nothing to unwind — which matters once the semantic phase adds its own storage; see [Semantic and hybrid phases](#semantic-and-hybrid-phases) for how the two index families coexist.
- **Per-entity searchers merged in Rust**, not one 9-way SQL `UNION ALL` and not a projection table (yet). Each entity has a *different* authorization join path; separate searchers own their joins — each a `Searcher` trait implementation with a uniform signature taking the shared `Scope` plus a single `domain::search::Filters` struct (the compiled, validated form of `IndexParams`) — are independently testable, and the `types` filter is literally "skip implementations". Each searcher reads only the `Filters` fields relevant to it (the action searcher reads `goal_id`/`goal_filter`/`status`, the topic searcher reads `topic_status`, and so on), so adding a param touches the struct and the concerned searchers, never nine signatures. Each searcher fetches up to `limit + 1` rows (see the pagination decision below for why the fetch depth must track the page size), results are merged by rank in the domain layer, and the global limit is clamped at 100 (the existing `MAX_LIMIT` precedent). A unified projection table arrives with the semantic phase, where it is unavoidable anyway.
- **Authorization is embedded in the queries, not request-layer gating.** Per-resource gating — the legacy `protect::*` layers and the access extractors replacing them (e.g. `web/src/extractors/coaching_session_access.rs`) — keys off a single resource id in the path; a search request has none, and neither mechanism can express "everything visible to this caller". Search still follows the extractor pattern at the caller end: the `Scope` is compiled from `AuthenticatedUser`'s preloaded roles (zero extra queries) and exposed as its own `FromRequestParts` extractor, so the `/search` handler receives its authorization input the same way new-style resource handlers receive their access proofs — no `protect::*` layer involved. The scope is then embedded in each searcher's SQL — the same inline-authorization precedent as `user_controller::index`.
- **Disallowed `types` are silently dropped, never 403.** This matches the codebase convention that an inaccessible resource looks identical to a missing one (`web/src/extractors/mod.rs::not_found`), lets the frontend ship one static type list for all roles, and prevents probing role boundaries. Only lexically unknown tokens (e.g. `types=gaols`) are a 400 — typos are caller bugs, not permission questions.
- **Single ranked list, not grouped-by-type.** Relevance interleaving across types is only possible server-side; grouping is a trivial client-side `groupBy(hit.type)`, while re-ranking a grouped response client-side is impossible (scores are only comparable within one response).
- **Keyset cursor pagination from day one.** This endpoint sets the codebase's first pagination precedent; keyset over `(score DESC, type ASC, id ASC)` doesn't paint us into a corner the way offset would. No `total_count` — a count over the full scoped corpus is the most expensive query in the feature and neither consumer needs it.
  Mechanics: every searcher applies the same continuation predicate in its own SQL and fetches up to `limit + 1` rows. Because the sort is mixed-direction (`score DESC, type ASC, id ASC`), the predicate cannot be a row-constructor comparison — `(score, type, id) < cursor` would run the tie-breakers backwards at equal scores, re-returning already-seen rows and permanently skipping valid later ones. The correct form is `score < :s OR (score = :s AND (type > :t OR (type = :t AND id > :i)))`; within one searcher `type` is a constant, so the type comparison folds to a static include-ties / exclude-ties / compare-ids decision per searcher. The cursor must round-trip `score` bit-exactly (encode the raw f32 bits, not a decimal rendering), or the equality arm misfires. In code the cursor is therefore a strong type, not a passed-around string: `Cursor { score: f32, hit_type: HitType, id: Id }` with explicit `encode() -> String` / `decode(&str) -> Result<Cursor, _>` methods (base64; `score` serialized via `f32::to_bits`/`from_bits`, which is what makes the bit-exact guarantee enforceable in exactly one place). The raw string exists only at the API boundary — opaque to clients, so the encoding can evolve without a breaking change — and a `decode` failure is the documented 400 for a malformed cursor. The encoded payload leads with a one-byte mode/version discriminator from day one: this predicate-based mechanism is specifically the *keyword-mode* pagination story, and the later modes paginate differently (a position within a frozen fused pool — see [Semantic and hybrid phases](#semantic-and-hybrid-phases)), so `decode` must be able to branch on payload shape rather than discover at PR 6 time that the encoding cannot express it. After the predicate, the domain merges, returns the top `limit`, and emits `next_cursor` from the last returned row only when the merged pool exceeds `limit`. The per-searcher fetch depth **must be `limit + 1`, never a smaller fixed cap**: with a shallower cap, one type's capped-out rows can sort above a cursor set by another type's hits and become permanently unreachable on the following page (and an under-filled merged pool would emit a false `next_cursor: null`). With `limit + 1` fetched per searcher, at most `limit` of a searcher's rows can make the page, so its deepest fetched row always sorts below the cursor, and every row it omitted sorts below that — still reachable. A per-type composite cursor would also solve this but costs more complexity than fetching deeper.

## API Contract

### Endpoint

```
GET /search
```

Standard extractors: `CompareApiVersion` (`X-Version` header), `AuthenticatedUser` (roles preloaded), cookie session auth, plus a search-specific `Scope` extractor (see [Authorization scoping](#authorization-scoping)). Registered in `web/src/router.rs` with utoipa path + schema registration, behind `require_auth` and a new `ThrottlePolicy::SEARCH_ENDPOINT` per-IP throttle (burst headroom for search-as-you-type; roughly `period_secs: 1, burst: 10`).

### Request parameters (`web/src/params/search.rs::IndexParams`)

| Param | Type | Semantics |
|---|---|---|
| `q` | string, **required** | Trimmed. Fewer than 2 chars after trim → 400. Longer than 256 chars → silently truncated (clamp precedent). Supports `websearch_to_tsquery` syntax: `"quoted phrase"`, `-negation`, `or`. |
| `types` | string, optional | Comma-separated entity types: `coaching_sessions,notes,transcripts,goals,actions,agreements,topics,users,organizations`. Unknown token → 400. Valid-but-not-permitted → silently dropped. Omitted → all types the caller may search. |
| `organization_id` | uuid, optional | Narrow to one organization. Intersects with the caller's scope (can never widen it). |
| `user_id` | uuid, optional | "By user": matches the creator (`user_id` column) for notes/goals/actions/agreements/topics; a participant (coach or coachee) for sessions/transcripts; ignored for users/organizations. Intersect-only. |
| `coaching_session_id` | uuid, optional | Narrow to one session (MCP vocabulary parity). Applies to notes/actions/agreements/topics/transcripts and the session itself. |
| `goal_id` | uuid, optional | Actions linked to that goal (`actions.goal_id`, the same filter `GET /actions` already exposes) and the goal itself for the `goals` type; ignored for other types — pair with `types=actions` to search within one goal's actions. Intersect-only. |
| `goal_filter` | enum, optional | `all` (default) / `linked` / `unlinked` — actions by goal linkage (`actions.goal_id IS NOT NULL` / `IS NULL`). Mirrors the `assignee_filter=all\|assigned\|unassigned` precedent on `GET /users/:id/actions`. Ignored for non-action types. `goal_filter=unlinked` combined with `goal_id` is contradictory → 400. |
| `status` | `entity::status::Status`, optional | Deserialized into the existing enum (`not_started`/`in_progress`/`completed`/`on_hold`/`wont_do`) exactly as `GET /users/:id/actions` already does — invalid value → 400. Applies to goals and actions; ignored for other types. |
| `topic_status` | `entity::topic_status::Status`, optional | Topics carry a different status vocabulary (`open`/`discussed`/`deferred`), so they get their own typed param rather than a search-specific union enum. Applies to topics only. Both enums derive `ToSchema`, so the frontend mirror comes through the OpenAPI schema. |
| `created_from` / `created_to` | date (YYYY-MM-DD), optional | Half-open `created_at` window `[from, to + 1 day)`, interpreted in `tz`. |
| `updated_from` / `updated_to` | date, optional | Same for `updated_at`. |
| `tz` | IANA name, optional | Defaults to UTC. Invalid → 400 `invalid_timezone`. Reuses the `AT TIME ZONE` conversion pattern from `entity_api::coaching_session::SessionQueryOptions`. |
| `limit` | u16, optional | Default 25, silently clamped to 100. Upcast to u64 at the SeaORM boundary; sized to the domain rather than copying the `goal_progress.rs` u32 precedent. |
| `cursor` | string, optional | Opaque keyset cursor from a previous response's `next_cursor` (base64 of `{score, type, id}`). Malformed → 400. |
| `mode` | enum, optional | `keyword` (default). `semantic` and `hybrid` reserved — advertised in the OpenAPI schema from day one, rejected with 400 until their phases ship. |
| `keyword_weight` | float, optional | Keyword-vs-semantic preference for `mode=hybrid` (weighted RRF; see [Semantic and hybrid phases](#semantic-and-hybrid-phases)). Default `0.5` (plain RRF), validated to `[0.0, 1.0]` → 400 outside. Meaningful only with `mode=hybrid`; supplying it with another mode is contradictory → 400 (the `goal_filter=unlinked` + `goal_id` precedent). Like `mode`, advertised from day one and 400 until PR 7 ships; like `mode`, it must be identical across pages of one cursor walk since it determines the fused ordering of the frozen pool that hybrid pagination walks (see [Semantic and hybrid phases](#semantic-and-hybrid-phases)). |

### Error types for the 400s

Every 400 above is boundary validation, decided in `web` before the domain is called, so all of them surface through the existing `WebErrorKind` enum (`web/src/error.rs`) — no new top-level error type, and no new `domain`/`entity_api` error kinds. They cannot ride the domain error chain anyway: `DomainErrorKind::Validation` maps to 422, not 400, so raising these below the web layer would silently change their documented status codes. (`websearch_to_tsquery` never throws on user input, so there is no query-parse error to bubble up from `entity_api` either.)

Concretely:

- **`tz`** reuses `WebErrorKind::InvalidTimezone` verbatim.
- **`status` / `topic_status`** are rejected by serde during `Query<IndexParams>` deserialization, exactly as `GET /users/:id/actions` already handles an invalid `status` — no error variant involved.
- **The rest** (`q` too short, unknown `types` token, `goal_filter` + `goal_id` contradiction, malformed `cursor`, unavailable `mode`, `keyword_weight` out of range or off-mode) share one new generic variant modeled on the `InvalidTimezone` shape: `WebErrorKind::InvalidParam { error: &'static str, message: String }`, rendering the same structured `{status_code: 400, error, message}` body with a stable discriminator per failure (`query_too_short`, `unknown_type`, `contradictory_params`, `malformed_cursor`, `mode_unavailable`, `keyword_weight_out_of_range`). One variant carrying context in fields follows the Error Variant Reuse rule in `.claude/coding-standards.md` (same status code + same caller handling = same variant), keeps `WebErrorKind` from growing a case per validation rule, and still gives the frontend a deterministic string to branch on; the web tests pin each discriminator. The variant is deliberately not search-specific — it is the generalization of what `InvalidTimezone` hard-codes, reusable by any future endpoint needing a discriminated 400. `Cursor::decode`'s `Err` maps to it at the controller, so a malformed cursor never routes through the domain's 422 path.

No new error enums are required; the only new enums the plan introduces are param vocabulary (`mode`, `goal_filter`, the `types` tokens), which fail at serde/validation time rather than as error types.

### Response shape

Wrapped in the standard `ApiResponse { status_code, data }` envelope.

```rust
// domain/src/search.rs
#[derive(Serialize, ToSchema)]
pub struct Results {
    pub query: String,              // trimmed, post-clamp — what was actually searched
    pub limit: u16,                 // post-clamp
    pub hits: Vec<Hit>,             // ordered by (score desc, type, id)
    pub next_cursor: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Hit {
    CoachingSession(SessionHit),
    Note(NoteHit),
    Transcript(TranscriptHit),
    Goal(GoalHit),
    Action(ActionHit),
    Agreement(AgreementHit),
    Topic(TopicHit),
    User(UserHit),
    Organization(OrganizationHit),
}
```

Every variant carries a common core plus per-type navigation context:

```rust
// Common core, #[serde(flatten)]ed into each hit struct
pub struct Core {
    pub id: Id,
    /// ts_rank; higher is better; comparable only within one response.
    pub score: f32,
    /// One-line label for the result row. Never empty: sessions use the
    /// composed display_title, with a deterministic final fallback of
    /// "Coaching session — YYYY-MM-DD" (defined below the variant table).
    pub title: String,
    /// Plain-text excerpt via ts_headline; matched terms wrapped in
    /// <mark>…</mark> markers. Not HTML — the FE splits on the markers and
    /// renders text nodes (no dangerouslySetInnerHTML).
    pub snippet: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: Option<DateTimeWithTimeZone>,
    pub organization_id: Id, // omitted on UserHit/OrganizationHit
}
```

| Variant | Extra fields |
|---|---|
| `SessionHit` | `coaching_relationship_id`, `date`, `display_title` |
| `NoteHit` | `coaching_session_id`, `coaching_relationship_id`, `session_date`, `session_display_title`, `author_user_id` |
| `TranscriptHit` | `transcription_id`, `coaching_session_id`, `coaching_relationship_id`, `session_date`, `start_ms`, `end_ms`, `speaker_label` — `id` is the segment row; `start_ms` is the playback deep-link offset |
| `GoalHit` | `coaching_relationship_id`, `status`, `created_in_session_id` |
| `ActionHit` | `coaching_session_id`, `coaching_relationship_id`, `goal_id` (nullable), `status`, `due_by`, `session_date`, `session_display_title` |
| `AgreementHit` | `coaching_session_id`, `coaching_relationship_id`, `session_date`, `session_display_title` |
| `TopicHit` | `coaching_session_id`, `coaching_relationship_id`, `status`, `priority` |
| `UserHit` | `email`, `first_name`, `last_name`, `display_name`, `organization_ids` — restricted to the intersection of the user's orgs and the requester's admin scope (no membership leakage) |
| `OrganizationHit` | `name`, `slug` |

`session_display_title` is hydrated post-query via the existing `entity_api::coaching_session_display_title::batch_load_display_titles` (it is computed, not a DB column). That composition — session title → first live topic body → first goal title — returns `None` when every tier is absent or blank, so search defines a deterministic final fallback: **`Coaching session — YYYY-MM-DD`**, formatted from the session's stored `date` (naive, no timezone conversion, so the label is identical for every caller). The fallback applies wherever the composed title surfaces: a `SessionHit`'s `title`/`display_title`, the `session_display_title` context field on note/action/agreement hits, and transcript hit titles that embed it. `title` is therefore never empty by construction, and implementations must not invent their own placeholder.

### Example

```
GET /search?q=quarterly%20review&types=notes,transcripts&limit=25
```

```json
{
  "status_code": 200,
  "data": {
    "query": "quarterly review",
    "limit": 25,
    "hits": [
      {
        "type": "note",
        "id": "0d9c…",
        "score": 0.91,
        "title": "Note by Barbara Coach",
        "snippet": "…prepare for the <mark>quarterly</mark> <mark>review</mark> by drafting…",
        "created_at": "2026-07-02T14:03:00Z",
        "updated_at": "2026-07-02T15:11:00Z",
        "organization_id": "a1b2…",
        "coaching_session_id": "9f3e…",
        "coaching_relationship_id": "77aa…",
        "session_date": "2026-07-02T13:00:00",
        "session_display_title": "Career growth — Jul 2, 2026",
        "author_user_id": "c4d5…"
      },
      {
        "type": "transcript",
        "id": "5e6f…",
        "score": 0.74,
        "title": "Transcript — Career growth — Jul 2, 2026",
        "snippet": "so for the <mark>quarterly</mark> <mark>review</mark> I think we should",
        "created_at": "2026-07-02T14:20:00Z",
        "updated_at": null,
        "organization_id": "a1b2…",
        "transcription_id": "1122…",
        "coaching_session_id": "9f3e…",
        "coaching_relationship_id": "77aa…",
        "session_date": "2026-07-02T13:00:00",
        "start_ms": 84200,
        "end_ms": 91500,
        "speaker_label": "Speaker A"
      }
    ],
    "next_cursor": null
  }
}
```

Clients must ignore unknown `type` values so new hit variants can ship without an `X-Version` bump.

## Authorization scoping

The controller derives the caller's scope in memory from `AuthenticatedUser(user).roles` (already preloaded — zero extra queries):

```rust
// domain/src/search.rs
pub struct Scope {
    pub user_id: Id,
    pub is_super_admin: bool,      // SuperAdmin role with organization_id NULL
    pub admin_org_ids: Vec<Id>,    // orgs where role == Admin
    pub member_org_ids: Vec<Id>,   // all orgs with any role row
}
```

Relationship-anchored searchers scope through a shared `visible_relationships` scope, rendered from `coaching_relationships::visible_to(scope)` - the query-form twin of `grants_access_to` (participation **and** current org membership) plus the admin tier:

```sql
WITH visible_relationships AS (
  SELECT cr.id, cr.organization_id
  FROM refactor_platform.coaching_relationships cr
  WHERE
    -- tier 1: participant AND currently a member of the relationship's org
    ((cr.coach_id = $user_id OR cr.coachee_id = $user_id)
       AND cr.organization_id = ANY($member_org_ids))
    -- tier 2: org admin sees every relationship in their org(s)
    OR cr.organization_id = ANY($admin_org_ids)
    -- tier 3 (super admin): this WHERE clause is omitted entirely
)
```

Per-entity anchoring:

| Type | Scope path |
|---|---|
| coaching_sessions, goals | `coaching_relationship_id IN (SELECT id FROM visible_relationships)` |
| notes, actions, agreements, topics | join `coaching_sessions` on `coaching_session_id`, then relationship-in-visible; topics add `deleted_at IS NULL` |
| transcripts | `transcript_segments → transcriptions → coaching_sessions`, then relationship-in-visible |
| users | admins only; membership via `user_roles.organization_id = ANY($admin_org_ids)` (so members without a relationship yet are still findable); super admin unrestricted |
| organizations | super admin only; `archived_at IS NULL` |

`user_id=` and `organization_id=` filters are additional `AND`s on rows already inside the visibility scope — safe by construction, they can never widen access.

### Duplication and drift

Query-embedded scoping means the participant rule exists in **two** forms: `grants_access_to` (in-memory Rust, one relationship at a time) and `coaching_relationships::visible_to` (query form, corpus-wide). The re-expression is forced by the problem shape — a row-by-row Rust predicate cannot drive an indexed, `LIMIT`ed, paginated query, and post-filtering fetched rows through `grants_access_to` breaks limits and cursors (unbounded over-fetch to fill a page). Eliminating the second copy entirely would mean either making every authorization check a DB round-trip (`SELECT EXISTS`) or adopting row-level security — both far bigger trades than this feature justifies. The duplication is instead kept minimal and guarded:

- **One condition, not nine.** The rule's query form lives on the entity as `coaching_relationships::visible_to(scope) -> Condition`, right beside `grants_access_to`, and search renders `visible_relationships` from it. Searchers never write their own scoping SQL; their per-entity part is only the FK path to a relationship id. Future corpus-wide consumers (the MCP tool, exports/reporting) reuse `visible_to` instead of minting a third copy, and the existing extractors can converge on it later — explicitly out of scope here.
- **Tier logic is not re-implemented in SQL.** `is_super_admin`, `admin_org_ids`, and `member_org_ids` are derived in Rust, once, from the same preloaded `roles` the extractors already use, and enter the SQL only as bind parameters. The condition expresses only the structural rule.
- **Equivalence is pinned by a test.** A DB-backed test seeds the full access matrix — participant/non-participant × current member/removed member/org admin/super admin, across two orgs — and asserts, for every (user, relationship) pair, that `grants_access_to` agrees with membership in `visible_to`'s result set. An edit to either expression that isn't mirrored in the other fails CI rather than shipping a leak or a regression.

This mirrors an accepted pattern elsewhere in the codebase: the "one role per user per org" invariant exists both in `user_roles::before_save` (Rust) and as partial unique indexes (SQL) — two expressions of one rule, because each runtime needs its own.

Only two searched types don't inherit the relationship rule, because they don't hang off a relationship: users (membership-based — admins of the user's orgs, via `user_roles`) and organizations (super admin only). Both are genuinely different, simpler policies with their own single homes, not copies. A topic's `deleted_at` or an organization's `archived_at` is lifecycle filtering, not authorization.

## Layer responsibilities

| Layer | Module | Responsibility |
|---|---|---|
| `entity` | `coaching_relationships.rs` | `visible_to(scope) -> Condition` — the participant rule's query form, defined beside `grants_access_to`. `search_chunks` entity added in the semantic phase. |
| `entity_api` | `search/mod.rs` + `search/{coaching_session,goal,action,agreement,topic,note,transcript,user,organization}.rs` | The `Searcher` trait, the `Filters` and `Request` structs it takes (see [the sketch below](#the-searcher-trait-and-its-inputs)), and the per-entity implementations: FTS expression constants (shared with index DDL), scope-aware WHERE clauses, `ts_rank`/`ts_headline`, per-searcher `limit + 1` fetch, keyset predicate |
| `domain` | `search.rs` | `Scope` derivation from roles, compiling `IndexParams` into `Filters` (declared in `entity_api::search`, re-exported here per the type re-export boundary), concurrent fan-out to searchers, merge + rank + clamp, cursor encode/decode, `display_title` hydration, `Results`/`Hit` types |
| `web` | `extractors/scope.rs` | `FromRequestParts` extractor wrapping `AuthenticatedUser` → compiled `Scope`, per the access-extractor pattern (`extractors/*_access.rs`) |
| `web` | `params/search.rs` | `IndexParams`, comma-split `types` parsing, clamps, utoipa `IntoParams`; compiles into `domain::search::Filters` |
| `web` | `controller/search_controller.rs` | `GET /search` handler, `ApiResponse` envelope, utoipa path |
| `web` | `router.rs` | Route + OpenAPI registration + `ThrottlePolicy::SEARCH_ENDPOINT` layer |
| `web` | `mcp/tools/*` (PR 4) | `search` MCP tool reusing `domain::search` with PAT-derived identity |
| `migration` | one migration per index PR | GIN expression indexes (fenced SQL below) |

### The `Searcher` trait and its inputs

The "uniform signature" concretely: the trait takes one shared request struct rather than a parameter list. `Filters` is the union of every filter param — that union costs nothing at the signature level, because a searcher ignoring a field is simply a no-op read.

```rust
// entity_api/src/search/mod.rs
pub struct Filters {
    // the compiled, validated form of IndexParams — union of all filter params
    pub organization_id: Option<Id>,
    pub user_id: Option<Id>,
    pub coaching_session_id: Option<Id>,
    pub goal_id: Option<Id>,
    pub goal_filter: GoalFilter,
    pub status: Option<Status>,
    pub topic_status: Option<TopicStatus>,
    pub created: Option<DateTimeRange>,   // tz already applied
    pub updated: Option<DateTimeRange>,
}

pub struct Request<'a> {
    pub q: &'a str,              // trimmed, clamped
    pub scope: &'a Scope,
    pub filters: &'a Filters,    // each searcher reads only its relevant fields
    pub cursor: Option<&'a Cursor>,
    pub fetch: u64,              // limit + 1
}

#[async_trait]
pub trait Searcher {
    fn hit_type(&self) -> HitType;
    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error>;
}
```

Nine unit structs implement the trait; the domain fans out over the full set and the `types` filter skips implementations before dispatch. Layering note: the trait consumes `Filters`, so the struct is **declared in `entity_api::search` and re-exported up through `domain`'s `lib.rs`** — `entity_api` cannot import `domain` types, and web still imports it as `domain::search::Filters`, so references to that path elsewhere in this plan describe the web-visible import, not the declaration site. `Scope` follows the same pattern, since `entity`'s `visible_to(scope)` already consumes it below `entity_api`.

## Migrations

Phase 1 — `migration/src/m20260825_000000_add_search_fts_indexes.rs`, via `execute_unprepared`:

```sql
CREATE INDEX idx_coaching_sessions_title_fts ON refactor_platform.coaching_sessions
  USING GIN (to_tsvector('english', coalesce(title, '')));
CREATE INDEX idx_goals_fts ON refactor_platform.goals
  USING GIN (to_tsvector('english', coalesce(title,'') || ' ' || coalesce(body,'')));
CREATE INDEX idx_actions_body_fts ON refactor_platform.actions
  USING GIN (to_tsvector('english', coalesce(body, '')));
CREATE INDEX idx_agreements_body_fts ON refactor_platform.agreements
  USING GIN (to_tsvector('english', coalesce(body, '')));
CREATE INDEX idx_coaching_session_topics_body_fts ON refactor_platform.coaching_session_topics
  USING GIN (to_tsvector('english', coalesce(body, '')));
```

PR 2 — transcript index, separate migration:

```sql
CREATE INDEX IF NOT EXISTS idx_transcript_segments_text_fts ON refactor_platform.transcript_segments
  USING GIN (to_tsvector('english', text));
```

**`CONCURRENTLY` caveat:** sea-orm-migration wraps each migration in a transaction, and `CREATE INDEX CONCURRENTLY` cannot run inside one. Decision: plain `CREATE INDEX` and accept the brief write lock — every phase-1 table is small. `transcript_segments` is the only potentially large table; if production size makes the lock unacceptable at PR-2 time, the fallback is a manual `CREATE INDEX CONCURRENTLY` run as `doadmin` beforehand, with the migration's `IF NOT EXISTS` making it a no-op. Users and organizations get no new index.

No new Postgres types in phase 1. The semantic phase adds `CREATE EXTENSION vector` — the `ALTER TYPE … OWNER TO refactor` rule applies to any type created there.

## Notes (TipTap content)

The live collaborative note document is **not in Postgres** — it lives in Tiptap Cloud, keyed by `coaching_sessions.collab_document_name`, and nothing in the backend ever reads it back: the Tiptap gateway (`domain/src/gateway/tiptap.rs`) only creates and deletes documents, and note editing goes through the collab token flow, never through the `/notes` CRUD endpoints. The `notes` table and its `body` column are a pre-Tiptap artifact — empty or stale for anything authored in the Tiptap era. There is no mirror to search.

- **Phase 1 therefore does not search notes at all.** Indexing `notes.body` would return essentially nothing while implying notes are covered. The `notes` value stays in the `types` vocabulary from day one (it is a known token, never a 400), but yields no hits until PR 5.
- **PR 5**: notes search activates against a real projection, designed against the planned **self-hosted `docs-collab-server`** (an in-repo replacement for Tiptap Cloud deployed alongside the stack). Two design points to settle with that workspace member:
  - **Where the plain text lives.** The collab server will persist documents in its own table(s); the projection may be a column or sibling table maintained by its persistence hook (flatten the ProseMirror JSON node tree to plain text on document store) rather than a standalone `note_documents` table owned by search. Decide once the collab server's schema exists — the searcher only needs `(coaching_session_id, plain_text)` with an FTS index, wherever that lives. The legacy `notes` table is not the target; its retirement can be handled separately.
  - **One-time import.** At cutover, existing documents must be exported from Tiptap Cloud into the collab server's store (fetch via the existing gateway pattern, `GET /api/documents/{name}?format=json`, for every session with a `collab_document_name`). Search piggybacks on that import — flattening happens as documents land — rather than running its own backfill.

## Transcript search granularity

Segment-level matching, session-level grouping:

- Match `transcript_segments.text`; return grouped hits — the top 3 matching segments per transcription via `ROW_NUMBER() OVER (PARTITION BY transcription_id ORDER BY rank DESC)`, each with its `ts_headline` snippet and `start_ms` deep link.
- Group rank = max segment rank (not sum, which would favor long rambly sessions).
- Ships as its own PR: `transcript_segments` is the only genuinely large table (hundreds to thousands of rows per session), so its index build and query tuning deserve isolation.

## Semantic and hybrid phases

Both are committed phases, not options.

- **Storage**: `CREATE EXTENSION vector` (pgvector; available on DigitalOcean managed Postgres, run as `doadmin`) + a `search_chunks` projection table: `(id, entity_type, entity_id, coaching_session_id?, organization_id, chunk_index, text, embedding vector(1536), embedding_model, tsv tsvector generated, created_at, updated_at)` with an HNSW index (`USING hnsw (embedding vector_cosine_ops)`). `vector(1536)` is the *current* model's dimensionality, not a schema-level commitment to one provider — the typmod is fixed per column because HNSW requires it, and the versioning bullet below covers what happens when the model changes.
- **Coexistence with the phase-1 GIN indexes**: the two index families coexist — semantic joins keyword, it does not replace it. `mode=keyword` stays the always-available deterministic path (no embedding call, works when the provider is down), and `mode=hybrid` *requires* both retrievals by definition: FTS ranking plus vector similarity, merged with RRF. They also differ in freshness. The per-table GIN indexes update inside the writing transaction, so keyword results are exact and current; `search_chunks` is populated by an async ingestion job (embedding calls are slow external requests), so anything served from it can briefly lag a write. Two end states are possible, decided at hybrid-phase time with real ingestion-latency data:
  - **A — permanent coexistence** (likely default): GIN on source tables serves keyword, HNSW on `search_chunks` serves semantic, hybrid uses both. Cost: two index families, slight ranking non-uniformity between modes.
  - **B — consolidation**: keyword re-points at the generated `search_chunks.tsv` column and the per-table GIN indexes are dropped in a two-line cleanup migration. Buys a uniform corpus and ranking for both retrievals; costs keyword search its just-typed-now-searchable freshness. Cheap to do later precisely because phase 1 chose expression indexes over stored columns — there is no schema to unwind.
- **Embedding provider**: a trait following the `meeting-ai/` provider-abstraction precedent — `EmbeddingProvider { async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> }` — with the concrete provider injected via config like other gateways. No provider dependency in `entity_api`.
- **Embedding-model versioning**: vectors from different models — or different versions of one model — are not comparable, so a mixed-model index silently returns garbage similarity rather than an error. Every chunk therefore records the `embedding_model` (provider/model name plus version, e.g. `openai/text-embedding-3-small@1`) that produced its vector, and retrieval always filters `WHERE embedding_model = $current`, so rows from two models are never ranked against each other. That column is what makes the `EmbeddingProvider` trait's swappability real at the storage layer:
  - **Same-dimension model swap** (the common case — provider families tend to share dims across versions): run the backfill job in re-embed mode — it re-embeds every chunk whose `embedding_model` differs from the configured target — and flip the retrieval filter to the new model only once the backfill completes, so the semantic corpus stays complete on the old model throughout the roll instead of serving a growing partial one. No schema change, no downtime.
  - **Dimension-changing swap**: additionally a real migration — new `vector(n)` column (or table) plus an HNSW rebuild — since the typmod cannot be altered in place. The column doesn't make this case free; it makes it survivable and incremental, with the same backfill-then-flip sequencing.
- **Chunking**: whole-record for titles/goals/actions/agreements/topics; ~300–500-token windows with ~15% overlap for note plain-text and transcripts (transcript chunks are contiguous segment runs preserving the first segment's `start_ms` for deep links).
- **Ingestion**: embed on create/update via a background job plus a batch backfill command.
- **Modes**: `mode=semantic` returns vector-similarity hits; `mode=hybrid` runs both retrievals and merges with Reciprocal Rank Fusion (RRF, k=60) — no cross-scorer normalization headaches. Same response shape in all modes; the same visibility scope is enforced (the chunk table carries the ids needed to join it), so no mode can leak a row another mode hides — but for the HNSW path that scope join is a *post-filter*, which raises a recall problem the next bullet addresses.
- **ANN recall under selective scopes**: an HNSW scan returns the approximate top-k nearest neighbors of the *whole* index, and Postgres applies the visibility WHERE clause to those candidates afterward. A regular user's scope is a tiny slice of a mature corpus (one or two relationships out of every org's data), so all k global neighbors can be rows they cannot see — the query returns zero hits even though visible matches exist. The keyword path is immune (GIN intersects the match set with the scope condition *before* ranking); the semantic path must mitigate explicitly, and the plan commits to a layered pair:
  - **Primary — iterative index scans** (pgvector ≥ 0.8, which becomes an explicit PR 6 prerequisite to verify against the managed-Postgres offering): `SET LOCAL hnsw.iterative_scan = relaxed_order`, bounded by `hnsw.max_scan_tuples`, so the index keeps walking until enough candidates survive the scope filter.
  - **Fallback and small-scope optimization — scoped exact scan**: the domain derives `Scope` before querying, so when the visible chunk count is small (precisely the population the problem hits), run an exact `ORDER BY embedding <=> $q` over the pre-filtered visible set — fast at that size and perfect recall, no ANN involved. The count threshold is tuned at PR 6 time.
  - Raising `hnsw.ef_search` alone was considered and set aside as the relied-upon fix: it is a fixed bound, so it shifts the failure point rather than removing it.
- **Pagination in semantic and hybrid modes is bounded-depth by design** — the keyword keyset predicate does not transfer, so these modes get their own mechanism rather than an implied reuse. A "resume after distance" predicate cannot ride the HNSW index, which only serves `ORDER BY embedding <=> $q LIMIT k`; resuming means walking deeper and skipping. Hybrid is worse: an RRF-fused score is a function of an item's rank *within the retrieved lists*, so fetching deeper lists on a later page would re-rank rows already returned — the keyset invariant (every row below the cursor stays below it) does not survive lists that deepen between pages. Both modes therefore retrieve a **fixed pool** per query (on the order of the top ~200 per retrieval arm; the exact depth is set at PR 6/7 time with data), fuse once over that frozen pool, and paginate within it. Freezing the depth is precisely what restores the invariant: fused scores cannot shift when the lists never deepen. `next_cursor` goes null at pool exhaustion — a documented bounded-depth contract for these modes, versus keyword's full-corpus walk. That asymmetry is a deliberate trade: deep paging of a relevance-ranked semantic result set has no real consumer (the MCP tool doesn't paginate at all), and every mainstream search engine bounds its deep-paging window for the same reason. The wire contract is untouched — the cursor is opaque, and its day-one mode/version discriminator lets semantic/hybrid cursors carry a pool-position payload instead of `{score, type, id}` without ambiguity. This is also the concrete reason `keyword_weight` (next bullet) must be constant across one cursor walk: it determines the frozen pool's fused ordering.
- **`keyword_weight` — the hybrid preference knob**: hybrid fusion is weighted RRF, `fused = w·1/(k + rank_keyword) + (1−w)·1/(k + rank_semantic)` with `w = keyword_weight`. The default `0.5` reduces exactly to plain RRF, so the parameter is backward-compatible by construction; a keyword-leaning caller sends `0.7`, a semantic-leaning one `0.3`. It is one param and two multipliers in the domain merge loop — no change to searchers, storage, or the response shape. Extreme values just degenerate toward single-mode results at hybrid's latency, so the useful range is narrow in practice; PR 7's relevance fixtures pin the fused ordering at the default and at representative off-center weights before the knob is documented as tuned.
- **Mode vocabulary stays a closed enum**: `hybrid` names a strategy the caller chose (run both retrievals, fuse deterministically), which is worth keeping distinct from an `auto` mode — a *delegation* where the server picks the strategy (and could, say, fall back to keyword when the embedding provider is down). `auto` is an anticipated additive fourth value, not a rename. An ordered-list form (`mode=keyword,semantic` as preference order) was considered and set aside: RRF is symmetric over rank positions, so "prefer one side" is a numeric weight, not an ordering — the knob is instead the explicit `keyword_weight` parameter on `mode=hybrid` (previous bullet), which expresses the preference continuously rather than as two coarse orderings. A list-valued mode would also make order load-bearing in validation, OpenAPI, and the mode-must-match-across-pages cursor contract, for no expressive power a weight doesn't give more precisely.

## Delivery plan

| PR | Scope | Size |
|---|---|---|
| **PR 0** | This plan document | XS |
| **PR 1 — Keyword search core** | FTS index migration; searchers for sessions, goals, actions, agreements, topics; `domain/src/search.rs` scope + merge/rank + cursor; `GET /search` controller + params + throttle + OpenAPI; all three visibility tiers; date/user/org/session/status/goal-linkage filters | L (~4–5 days) |
| **PR 2 — Transcript search** | Segment FTS index (CONCURRENTLY note), grouped-by-session searcher, `start_ms` deep links | M (~2–3 days) |
| **PR 3 — Members + organizations** | Users search (admin scopes, `ILIKE` + `LOWER(email)`), organizations search (super admin, active only), ranking/snippet polish | S–M (~1–2 days) |
| **PR 4 — MCP `search` tool** | Thin adapter over `domain::search` per the MCP filter vocabulary (`keyword`, optional `types`/`coaching_session_id`/`coachee_id`/`goal_id`/`goal_filter`/`status`/`topic_status`/`date_from`/`date_to`); PAT identity → same `Scope`; `tz` from `users.timezone`; limit default 10 clamp 25; no cursor (agents refine queries, not paginate); no users/orgs types (admin tools are post-MVP); output strips `score` and `<mark>` markers and adds a frontend `session_url` | S (~1 day) |
| **PR 5 — Notes search via docs-collab-server** | Plain-text projection in/alongside the collab server's document store, ProseMirror-JSON→text flattener, persistence-hook ingestion, one-time Tiptap Cloud import at cutover, notes searcher (activates the `notes` type) | M (~2–3 days) |
| **PR 6 — Semantic foundation** | pgvector extension, `search_chunks`, chunking, `EmbeddingProvider` trait + concrete provider, backfill job, `mode=semantic` | L (~5+ days) |
| **PR 7 — Hybrid mode** | `mode=hybrid` weighted-RRF merge, `keyword_weight` param, relevance evaluation fixtures | S–M (~1–2 days) |

## Testing strategy

- **entity (scoping)**: the `grants_access_to` ⇄ `visible_to` equivalence test described in [Duplication and drift](#duplication-and-drift) — the guard against the two expressions of the participant rule drifting apart.
- **entity_api (per searcher)**: DB-backed integration tests — FTS semantics (stemming, quoted phrases, negation), scope isolation (participant cannot see other relationships; removed-from-org user sees nothing; org admin sees the whole org and nothing outside it; super admin sees all), soft-delete/archive exclusion, timezone edge cases (reuse the `SessionQueryOptions` test style).
- **domain**: unit tests for `Scope` derivation from role fixtures, merge/rank/clamp/cursor determinism with fixed rank inputs (including equal-score groups split across a page boundary — exactly-once delivery in both directions of the tie-breakers), and the title fallback chain — including a session with no title, no topics, and no goals yielding exactly `Coaching session — YYYY-MM-DD`.
- **web**: controller tests pinning the `ApiResponse` envelope, the structured error shapes (`invalid_timezone`, 400s), the silent type-drop behavior (regular user requesting `types=users,organizations` gets 200 with those types absent), and clamping.
- **testing-tools**: a "searchable corpus" scenario builder in `scenarios.rs` (org + relationship + session + goal/action/agreement/topic/note/transcript segments seeded with known phrases), reused across PRs 1–5.
- **Semantic phase**: mock `EmbeddingProvider` with deterministic vectors; golden-set relevance fixtures for RRF ordering; and a scoped-recall scenario mirroring the leak tests from the miss side — seed a large corpus invisible to a tier-1 user plus a few visible semantically-matching chunks, and assert their semantic search returns those hits rather than coming back empty.

## Security considerations

- **No privilege escalation by construction**: every filter intersects the visibility scope; disallowed types drop silently; a `UserHit`'s `organization_ids` are intersected with the requester's admin scope so cross-org memberships never leak.
- **Rate limiting**: per-IP throttle with burst headroom on `/search`; MCP calls fall under the PAT-scoped limits already flagged in the MCP architecture doc. No per-keystroke DB writes (the `user_lookup` recording pattern exists for cross-org email enumeration, which search's org-scoped users type does not permit).
- **Prompt injection**: snippets returned to MCP clients contain user-generated text; as with all MCP tool output, sanitization is the client's responsibility (per the MCP architecture doc).
- **Score semantics**: `score` is documented as comparable only within a single response — never across queries or modes.
