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
| Notes | `body` (see [Notes](#notes-tiptap-content)) | Same |
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

**Filters**: `created_at`/`updated_at` date ranges (timezone-aware), by user, by organization, by coaching session, by status, by entity type.

### Relationship to Tiptap's search API

The product question "can we copy or emulate Tiptap's search API?" resolves to: **emulate the shape, do not use the product.**

- Tiptap Cloud's Semantic Search is an add-on that only indexes documents stored in Tiptap Cloud. It structurally cannot see our Postgres-resident content (transcripts, goals, actions, agreements, topics), which is most of the corpus.
- Its documentation page has been removed (404), and its request body is documented inconsistently across sources — it is not a stable dependency.
- Its *spirit* is right and we keep it: a single search call, `{query, limit}` in, a flat scored result array out. We extend that with the type discriminator, authorization scoping, and navigation context our consumers need.

This also stays correct as the platform moves from Tiptap Cloud to the planned self-hosted `docs-collab-server` workspace member (see [PR 5](#pr-5--notes-projection-via-docs-collab-server)).

## Decisions

- **Keyword search first, semantic later, both first-class.** Phase 1 ships a fast, deterministic, always-available keyword search on Postgres full-text search — no extensions, no embedding provider, no per-query cost. Semantic (`mode=semantic`) and hybrid (`mode=hybrid`) are committed later phases, not maybes: the API reserves the `mode` parameter with all three values from day one, defaulting to `keyword`, and the response shape is identical in every mode so adding modes is additive, never breaking.
- **`websearch_to_tsquery('english', q)`** rather than `plainto_tsquery`: it never errors on malformed user input and gives humans and LLMs `"quoted phrases"`, `-negation`, and `or` for free. `ILIKE '%q%'` is rejected for content search (cannot be indexed without `pg_trgm`, no ranking, no multi-word semantics). `pg_trgm` fuzzy matching is deferred as an optional add-on.
- **Exception — users and organizations** are searched with `ILIKE prefix%` on name/email fields plus the existing `LOWER(email)` index: the tables are small, and stemming hurts proper names. No new indexes needed.
- **GIN expression indexes**, not stored generated `tsvector` columns, for phase 1: no schema change, no entity regeneration, no write amplification. The tradeoff is that the query must repeat the index expression exactly, so the expression SQL text lives once as constants in `entity_api/src/search/` shared by index DDL documentation and query code. Revisit stored columns with `setweight` (title vs body weighting) only if ranking tuning demands it.
- **Per-entity searchers merged in Rust**, not one 9-way SQL `UNION ALL` and not a projection table (yet). Each entity has a *different* authorization join path; separate searchers own their joins, are independently testable, and the `types` filter is simply "skip searchers". Each searcher fetches up to `limit + 1` rows (see the pagination decision below for why the fetch depth must track the page size), results are merged by rank in the domain layer, and the global limit is clamped at 100 (the existing `MAX_LIMIT` precedent). A unified projection table arrives with the semantic phase, where it is unavoidable anyway.
- **Authorization is embedded in the queries, not middleware.** The `protect::*` route layers key off a single resource id and cannot express "everything visible to this caller". The searchers take a caller scope compiled from the preloaded roles (zero extra queries) — the same inline-authorization precedent as `user_controller::index`.
- **Disallowed `types` are silently dropped, never 403.** This matches the codebase convention that an inaccessible resource looks identical to a missing one (`web/src/extractors/mod.rs::not_found`), lets the frontend ship one static type list for all roles, and prevents probing role boundaries. Only lexically unknown tokens (e.g. `types=gaols`) are a 400 — typos are caller bugs, not permission questions.
- **Single ranked list, not grouped-by-type.** Relevance interleaving across types is only possible server-side; grouping is a trivial client-side `groupBy(hit.type)`, while re-ranking a grouped response client-side is impossible (scores are only comparable within one response).
- **Keyset cursor pagination from day one.** This endpoint sets the codebase's first pagination precedent; keyset over `(score DESC, type ASC, id ASC)` doesn't paint us into a corner the way offset would. No `total_count` — a count over the full scoped corpus is the most expensive query in the feature and neither consumer needs it.
  Mechanics: every searcher applies the same continuation predicate in its own SQL and fetches up to `limit + 1` rows. Because the sort is mixed-direction (`score DESC, type ASC, id ASC`), the predicate cannot be a row-constructor comparison — `(score, type, id) < cursor` would run the tie-breakers backwards at equal scores, re-returning already-seen rows and permanently skipping valid later ones. The correct form is `score < :s OR (score = :s AND (type > :t OR (type = :t AND id > :i)))`; within one searcher `type` is a constant, so the type comparison folds to a static include-ties / exclude-ties / compare-ids decision per searcher. The cursor must round-trip `score` bit-exactly (encode the raw f32 bits, not a decimal rendering), or the equality arm misfires. After the predicate, the domain merges, returns the top `limit`, and emits `next_cursor` from the last returned row only when the merged pool exceeds `limit`. The per-searcher fetch depth **must be `limit + 1`, never a smaller fixed cap**: with a shallower cap, one type's capped-out rows can sort above a cursor set by another type's hits and become permanently unreachable on the following page (and an under-filled merged pool would emit a false `next_cursor: null`). With `limit + 1` fetched per searcher, at most `limit` of a searcher's rows can make the page, so its deepest fetched row always sorts below the cursor, and every row it omitted sorts below that — still reachable. A per-type composite cursor would also solve this but costs more complexity than fetching deeper.

## API Contract

### Endpoint

```
GET /search
```

Standard extractors: `CompareApiVersion` (`X-Version` header), `AuthenticatedUser` (roles preloaded), cookie session auth. Registered in `web/src/router.rs` with utoipa path + schema registration, behind `require_auth` and a new `ThrottlePolicy::SEARCH_ENDPOINT` per-IP throttle (burst headroom for search-as-you-type; roughly `period_secs: 1, burst: 10`).

### Request parameters (`web/src/params/search.rs::IndexParams`)

| Param | Type | Semantics |
|---|---|---|
| `q` | string, **required** | Trimmed. Fewer than 2 chars after trim → 400. Longer than 256 chars → silently truncated (clamp precedent). Supports `websearch_to_tsquery` syntax: `"quoted phrase"`, `-negation`, `or`. |
| `types` | string, optional | Comma-separated entity types: `coaching_sessions,notes,transcripts,goals,actions,agreements,topics,users,organizations`. Unknown token → 400. Valid-but-not-permitted → silently dropped. Omitted → all types the caller may search. |
| `organization_id` | uuid, optional | Narrow to one organization. Intersects with the caller's scope (can never widen it). |
| `user_id` | uuid, optional | "By user": matches the creator (`user_id` column) for notes/goals/actions/agreements/topics; a participant (coach or coachee) for sessions/transcripts; ignored for users/organizations. Intersect-only. |
| `coaching_session_id` | uuid, optional | Narrow to one session (MCP vocabulary parity). Applies to notes/actions/agreements/topics/transcripts and the session itself. |
| `status` | string, optional | Applies only to types that carry a status (goals, actions, topics). |
| `created_from` / `created_to` | date (YYYY-MM-DD), optional | Half-open `created_at` window `[from, to + 1 day)`, interpreted in `tz`. |
| `updated_from` / `updated_to` | date, optional | Same for `updated_at`. |
| `tz` | IANA name, optional | Defaults to UTC. Invalid → 400 `invalid_timezone`. Reuses the `AT TIME ZONE` conversion pattern from `entity_api::coaching_session::SessionQueryOptions`. |
| `limit` | u32, optional | Default 25, silently clamped to 100. |
| `cursor` | string, optional | Opaque keyset cursor from a previous response's `next_cursor` (base64 of `{score, type, id}`). Malformed → 400. |
| `mode` | enum, optional | `keyword` (default). `semantic` and `hybrid` reserved — advertised in the OpenAPI schema from day one, rejected with 400 until their phases ship. |

### Response shape

Wrapped in the standard `ApiResponse { status_code, data }` envelope.

```rust
// domain/src/search.rs
#[derive(Serialize, ToSchema)]
pub struct Results {
    pub query: String,              // trimmed, post-clamp — what was actually searched
    pub limit: u32,                 // post-clamp
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
| `ActionHit` | `coaching_session_id`, `coaching_relationship_id`, `status`, `due_by`, `session_date`, `session_display_title` |
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

Relationship-anchored searchers scope through a shared `visible_relationships` fragment — the SQL translation of `grants_access_to` (participation **and** current org membership) plus the admin tier:

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

## Layer responsibilities

| Layer | Module | Responsibility |
|---|---|---|
| `entity` | — | No changes in phase 1. `search_chunks` entity added in the semantic phase. |
| `entity_api` | `search/mod.rs` + `search/{coaching_session,goal,action,agreement,topic,note,transcript,user,organization}.rs` | Per-entity searchers: FTS expression constants (shared with index DDL), scope-aware WHERE clauses, `ts_rank`/`ts_headline`, per-searcher `limit + 1` fetch, keyset predicate |
| `domain` | `search.rs` | `Scope` derivation from roles, concurrent fan-out to searchers, merge + rank + clamp, cursor encode/decode, `display_title` hydration, `Results`/`Hit` types |
| `web` | `params/search.rs` | `IndexParams`, comma-split `types` parsing, clamps, utoipa `IntoParams` |
| `web` | `controller/search_controller.rs` | `GET /search` handler, `ApiResponse` envelope, utoipa path |
| `web` | `router.rs` | Route + OpenAPI registration + `ThrottlePolicy::SEARCH_ENDPOINT` layer |
| `web` | `mcp/tools/*` (PR 4) | `search` MCP tool reusing `domain::search` with PAT-derived identity |
| `migration` | one migration per index PR | GIN expression indexes (fenced SQL below) |

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
CREATE INDEX idx_notes_body_fts ON refactor_platform.notes
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

The live collaborative note document is **not in Postgres** — it lives in Tiptap Cloud, keyed by `coaching_sessions.collab_document_name`. Postgres holds a secondary `notes.body` mirror.

- **Phase 1**: search `notes.body` as best-effort. Results may lag the live doc; acceptable and honest for v1. Do not fetch from Tiptap during search (latency, rate limits, unindexable).
- **PR 5**: a proper projection, designed against the planned **self-hosted `docs-collab-server`** (an in-repo replacement for Tiptap Cloud deployed alongside the stack). Its persistence hook flattens the ProseMirror JSON node tree to plain text on document store and upserts into `note_documents(coaching_session_id, collab_document_name, plain_text, synced_at)`. This is simpler and more reliable than Tiptap Cloud webhooks (no external fetch, no missed-webhook reconciliation). If projection must ship before the collab server lands, an interim backfill fetches docs through the existing gateway (`domain/src/gateway/tiptap.rs`, `GET /api/documents/{name}?format=json`) — interim only, deleted when the collab server arrives.

## Transcript search granularity

Segment-level matching, session-level grouping:

- Match `transcript_segments.text`; return grouped hits — the top 3 matching segments per transcription via `ROW_NUMBER() OVER (PARTITION BY transcription_id ORDER BY rank DESC)`, each with its `ts_headline` snippet and `start_ms` deep link.
- Group rank = max segment rank (not sum, which would favor long rambly sessions).
- Ships as its own PR: `transcript_segments` is the only genuinely large table (hundreds to thousands of rows per session), so its index build and query tuning deserve isolation.

## Semantic and hybrid phases

Both are committed phases, not options.

- **Storage**: `CREATE EXTENSION vector` (pgvector; available on DigitalOcean managed Postgres, run as `doadmin`) + a `search_chunks` projection table: `(id, entity_type, entity_id, coaching_session_id?, organization_id, chunk_index, text, embedding vector(1536), tsv tsvector generated, created_at, updated_at)` with an HNSW index (`USING hnsw (embedding vector_cosine_ops)`). Keyword search may optionally re-point at `search_chunks.tsv` for uniform ranking once it exists.
- **Embedding provider**: a trait following the `meeting-ai/` provider-abstraction precedent — `EmbeddingProvider { async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> }` — with the concrete provider injected via config like other gateways. No provider dependency in `entity_api`.
- **Chunking**: whole-record for titles/goals/actions/agreements/topics; ~300–500-token windows with ~15% overlap for note plain-text and transcripts (transcript chunks are contiguous segment runs preserving the first segment's `start_ms` for deep links).
- **Ingestion**: embed on create/update via a background job plus a batch backfill command.
- **Modes**: `mode=semantic` returns vector-similarity hits; `mode=hybrid` runs both retrievals and merges with Reciprocal Rank Fusion (RRF, k=60) — no cross-scorer normalization headaches. Same response shape in all modes; authorization scoping applies identically (the chunk table carries the ids needed to join the same visibility scope).

## Delivery plan

| PR | Scope | Size |
|---|---|---|
| **PR 0** | This plan document | XS |
| **PR 1 — Keyword search core** | FTS index migration; searchers for sessions, goals, actions, agreements, topics, notes.body; `domain/src/search.rs` scope + merge/rank + cursor; `GET /search` controller + params + throttle + OpenAPI; all three visibility tiers; date/user/org/session/status filters | L (~4–5 days) |
| **PR 2 — Transcript search** | Segment FTS index (CONCURRENTLY note), grouped-by-session searcher, `start_ms` deep links | M (~2–3 days) |
| **PR 3 — Members + organizations** | Users search (admin scopes, `ILIKE` + `LOWER(email)`), organizations search (super admin, active only), ranking/snippet polish | S–M (~1–2 days) |
| **PR 4 — MCP `search` tool** | Thin adapter over `domain::search` per the MCP filter vocabulary (`keyword`, optional `types`/`coaching_session_id`/`coachee_id`/`status`/`date_from`/`date_to`); PAT identity → same `Scope`; `tz` from `users.timezone`; limit default 10 clamp 25; no cursor (agents refine queries, not paginate); no users/orgs types (admin tools are post-MVP); output strips `score` and `<mark>` markers and adds a frontend `session_url` | S (~1 day) |
| **PR 5 — Notes projection via docs-collab-server** | `note_documents` table, ProseMirror-JSON→text flattener, persistence-hook ingestion (interim Tiptap Cloud backfill only if needed), notes searcher re-pointed at the projection | M (~2–3 days) |
| **PR 6 — Semantic foundation** | pgvector extension, `search_chunks`, chunking, `EmbeddingProvider` trait + concrete provider, backfill job, `mode=semantic` | L (~5+ days) |
| **PR 7 — Hybrid mode** | `mode=hybrid` RRF merge, relevance evaluation fixtures | S–M (~1–2 days) |

## Testing strategy

- **entity_api (per searcher)**: DB-backed integration tests — FTS semantics (stemming, quoted phrases, negation), scope isolation (participant cannot see other relationships; removed-from-org user sees nothing; org admin sees the whole org and nothing outside it; super admin sees all), soft-delete/archive exclusion, timezone edge cases (reuse the `SessionQueryOptions` test style).
- **domain**: unit tests for `Scope` derivation from role fixtures, merge/rank/clamp/cursor determinism with fixed rank inputs (including equal-score groups split across a page boundary — exactly-once delivery in both directions of the tie-breakers), and the title fallback chain — including a session with no title, no topics, and no goals yielding exactly `Coaching session — YYYY-MM-DD`.
- **web**: controller tests pinning the `ApiResponse` envelope, the structured error shapes (`invalid_timezone`, 400s), the silent type-drop behavior (regular user requesting `types=users,organizations` gets 200 with those types absent), and clamping.
- **testing-tools**: a "searchable corpus" scenario builder in `scenarios.rs` (org + relationship + session + goal/action/agreement/topic/note/transcript segments seeded with known phrases), reused across PRs 1–5.
- **Semantic phase**: mock `EmbeddingProvider` with deterministic vectors; golden-set relevance fixtures for RRF ordering.

## Security considerations

- **No privilege escalation by construction**: every filter intersects the visibility scope; disallowed types drop silently; a `UserHit`'s `organization_ids` are intersected with the requester's admin scope so cross-org memberships never leak.
- **Rate limiting**: per-IP throttle with burst headroom on `/search`; MCP calls fall under the PAT-scoped limits already flagged in the MCP architecture doc. No per-keystroke DB writes (the `user_lookup` recording pattern exists for cross-org email enumeration, which search's org-scoped users type does not permit).
- **Prompt injection**: snippets returned to MCP clients contain user-generated text; as with all MCP tool output, sanitization is the client's responsibility (per the MCP architecture doc).
- **Score semantics**: `score` is documented as comparable only within a single response — never across queries or modes.
