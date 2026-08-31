# Test Plan: Manually Testing the Search Capability

Verify that `GET /search` finds what it should, filters how it should, and —
above all — returns **nothing the caller could not already read directly**.
Companion to the
[search implementation plan](../implementation-plans/search-capability-backend.md).

The single most important property: **search is a new read path over old data,
so every authorization boundary that exists today must hold through it.** A
search hit, a snippet, a result count, or even a *difference in response shape*
that reveals the existence of another relationship's content is an access leak,
even if the full record is never returned.

> [!IMPORTANT]
> Silent dropping is part of the contract. A regular user requesting
> `types=users,organizations` gets **200 with those types absent**, not a 403.
> When a scenario below says "zero hits", a 403 or an error-shaped response is
> a **failure** — it leaks that the boundary exists and where it is.

> [!NOTE]
> Scenario applicability by delivery phase:
> A–H apply from **PR 1** (keyword core). I applies from **PR 2**
> (transcripts). J applies from **PR 3** (members/organizations). K applies
> from **PR 4** (MCP tool). Run the full plan again after each phase lands —
> every new searchable type is a new potential leak path.

## 1. Prerequisites

- Backend on the branch/preview deploying the phase under test.
- An API client with an authenticated cookie jar per persona (`curl -b`,
  Postman, or devtools on a logged-in tab). Every request needs the
  `X-Version` header.
- Two organizations and six personas:

| Persona | Role | Purpose |
|---|---|---|
| *Casey* | coach in **Acme**, relationship **R1** with Robin | the regular-user searcher |
| *Robin* | coachee in R1; also a plain member of **Globex** | multi-org regular user; later the removal subject |
| *Jordan* + *Morgan* | coach/coachee of relationship **R2** in Acme (no overlap with Casey) | the leak target inside the same org |
| *Alex* | **Admin** of Acme only; participates in no relationship | the org-admin tier |
| *Sam* | global **SuperAdmin** | the unrestricted tier |
| *Pat* | coach of relationship **G1** in Globex | the cross-org leak target |

- Seed each relationship's content with a unique sentinel word that appears
  nowhere else, in **every** searchable field:

| Sentinel | Where it must appear |
|---|---|
| `kumquat` | R1: session title, goal title *and* body, action body, agreement body, one live topic body, one **soft-deleted** topic body, transcript segments (PR 2+), note body via the collaborative editor (PR 5+) |
| `tamarind` | R2: same spread |
| `yuzu` | G1 (Globex): same spread |

- Additionally: one **archived** organization (*Initech*, `archived_at` set)
  and one Acme member with no relationship at all (for J).
- One **bare session** in R1: no title, no topics, no goals — containing only
  an action whose body includes `kumquat`. This is the only way to observe the
  display-title fallback, since a titleless session has no text of its own to
  match.

Record before starting:

| Thing | Where to find it |
|---|---|
| `organization_id` (Acme, Globex) | URL of each org page |
| `coaching_relationship_id` (R1, R2, G1) | Network tab on the sessions list |
| `coaching_session_id` (one per relationship) | URL of the session page |
| a R2 `coaching_session_id` | needed for the probing checks in D |

All example requests below elide the host and auth; append params to
`GET /search`.

## 2. Scenario A: response contract and envelope

Logged in as Casey. `GET /search?q=kumquat`.

| Check | Expected |
|---|---|
| HTTP status | 200 (transport), `"status_code": 200` in the body — the `ApiResponse` convention |
| Body shape | `data.query`, `data.limit`, `data.hits[]`, `data.next_cursor` |
| Every hit | has `type`, `id`, `score`, `title` (never empty), `created_at`, `organization_id` |
| Hit ordering | non-increasing `score` |
| Snippets | plain text with literal `<mark>…</mark>` markers around matched terms — check the raw JSON, and confirm the FE renders them as highlights, **not** as injected HTML |
| Session-anchored hits | carry `coaching_session_id`, `coaching_relationship_id`, `session_display_title` (non-empty even where the session's `title` column is NULL) |
| The bare session's action hit | `session_display_title` is exactly `Coaching session — YYYY-MM-DD` (the session's date) — not empty, not an improvised placeholder |

## 3. Scenario B: keyword semantics (feature requirements)

Still Casey. All against R1 content.

| Request | Expected |
|---|---|
| `q=kumquats` (plural) | same hits as `q=kumquat` — stemming works |
| `q="kumquat harvest"` where only "harvest kumquat" exists | no phrase match (quoted phrases are order-sensitive) |
| `q=kumquat -goal` (word `goal` present in the goal body) | goal hit disappears, others remain |
| `q=kumquat or tamarind` | identical to `q=kumquat` — the `or` arm matches nothing *visible*; see D |
| `q=` or `q=k` | 400 (min length 2 after trim) |
| `q=` + 300 chars | 200; `data.query` shows the truncated (≤256) string |
| `types=goals,actions` | only those hit types |
| `types=gaols` (typo) | **400** — lexically unknown token |
| `limit=3` | ≤3 hits, `next_cursor` non-null if more exist |
| `limit=5000` | clamped: `data.limit` is 100 |
| follow `next_cursor` until null | no duplicate `(type,id)` pairs across pages, no gaps |
| cap-vs-cursor regression: seed **one type with more matches than `limit`** (e.g. 30+ `kumquat` actions) alongside a handful of higher-scoring hits of other types, then walk all pages with a small `limit` | every seeded row eventually appears exactly once — the dominated type's overflow must not vanish after page 1, and `next_cursor` must not go null while matches remain |
| tie-breaker regression: seed rows with **identical text** (identical bodies produce identical scores), ideally in two different types (e.g. the same sentence in several actions *and* agreements), then walk pages with a `limit` that splits the tie group across a page boundary | each tied row appears exactly once — duplicates mean the continuation predicate re-selects rows from before the cursor; missing rows mean the tie-breakers run backwards |
| tampered/garbage `cursor` | 400 |
| `mode=keyword` | same as omitted |
| `mode=semantic` (before PR 6) | 400 |

## 4. Scenario C: date, user, and session filters

Still Casey. R1 needs content created on at least two distinct dates for this.

| Request | Expected |
|---|---|
| `created_from=<yesterday>&created_to=<yesterday>&tz=America/New_York` | only rows whose `created_at` falls on that New-York calendar day; the window is inclusive of the whole day |
| `tz=Not/AZone` | 400 with `"error": "invalid_timezone"` |
| `user_id=<Robin>` | only R1 content Robin authored (notes/goals/actions/agreements/topics) or participates in (sessions/transcripts) |
| `coaching_session_id=<R1 session>` | only content of that session plus the session itself |
| `organization_id=<Acme>` | unchanged (Casey's scope is already Acme-only) |

## 5. Scenario D: regular-user access leaks (the core of this plan)

Everything here runs as **Casey**. Every row's expectation is **200 with zero
matching hits** unless stated otherwise. Any hit, any error status, and any
response-shape difference from an ordinary empty result is a leak.

| Probe | Request | Leak being ruled out |
|---|---|---|
| Same-org, other relationship | `q=tamarind` | R2 content visible to a non-participant |
| Cross-org | `q=yuzu` | cross-organization visibility |
| Restricted types are dropped silently | `q=kumquat&types=users,organizations` | 200, zero hits, **not 403** — role boundary is not confirmed to the caller |
| Filter as a widening device | `q=tamarind&user_id=<Morgan>` | `user_id` must intersect scope, never expand it |
| Foreign org id | `q=yuzu&organization_id=<Globex>` | Robin *is* a Globex member but **Casey is not**; zero hits |
| Foreign session probe | `q=tamarind&coaching_session_id=<R2 session>` | existence of the session is not confirmed (empty result, not 404/403) |
| Snippet leak via `or` | `q=kumquat or tamarind` | hit list identical to plain `q=kumquat`; no snippet contains `tamarind` |
| Timing sanity (coarse) | compare `q=tamarind` vs `q=xyzzynonsense` | both empty; grossly different latency would hint the scoped query still scanned foreign rows — note it if observed |

Then the **multi-org member**: log in as Robin (member of Acme and Globex,
participant only in R1).

| Probe | Expected |
|---|---|
| `q=kumquat` | R1 hits (Robin is a participant) |
| `q=yuzu` | zero hits — org *membership* without relationship *participation* grants nothing |
| `q=kumquat&organization_id=<Globex>` | zero hits — org filter intersects, R1 is in Acme |

## 6. Scenario E: soft-delete and archive exclusions

| Probe | Request (as the broadest user who could see it) | Expected |
|---|---|---|
| Deleted topic | Casey: `q=kumquat&types=topics` | only the **live** topic; the soft-deleted one absent |
| Archived org (PR 3+) | Sam: `q=Initech&types=organizations` | zero hits (`archived_at IS NULL` filter) |

## 7. Scenario F: org-admin tier

Logged in as **Alex** (Acme admin, participant in nothing).

| Probe | Expected |
|---|---|
| `q=kumquat` | full R1 hit set — admin sees relationships they're not part of |
| `q=tamarind` | full R2 hit set |
| `q=yuzu` | **zero hits** — Alex administers Acme, not Globex |
| `q=kumquat&types=organizations` | zero hits, silently dropped — organizations remain super-admin-only |

## 8. Scenario G: removal revokes search access

Companion to
[remove_org_member_revokes_access_manual_testing.md](./remove_org_member_revokes_access_manual_testing.md) —
search must honor the same rule: participation without *current* membership
grants nothing.

1. As an Acme admin, remove **Robin** from Acme (`DELETE
   /organizations/{acme}/users/{robin}/role`).
2. As Robin (existing session cookie): `GET /search?q=kumquat`.

| Check | Expected |
|---|---|
| Hits | **zero** — Robin was a participant of R1, but is no longer an Acme member |
| Response | ordinary empty 200, indistinguishable from a no-match search |
| As Casey afterward | `q=kumquat` still returns the full R1 set — the data survived |

Re-add Robin and confirm `q=kumquat` returns hits again.

## 9. Scenario H: super-admin tier

Logged in as **Sam**.

| Probe | Expected |
|---|---|
| `q=kumquat`, `q=tamarind`, `q=yuzu` | all return their full hit sets |
| `q=<org name>&types=organizations` (PR 3+) | active organizations only |
| Sanity | Sam's hits for `q=kumquat` are a superset of Casey's, and each individual hit is identical in shape/fields to what a participant sees |

## 10. Scenario I: transcripts (PR 2+)

Seed R1's transcript with `kumquat` in at least four segments across two
different speakers, and `tamarind` in an R2 transcript.

| Check | Expected |
|---|---|
| Casey: `q=kumquat&types=transcripts` | grouped hits: at most **3 segments per transcription**, each with `transcription_id`, `start_ms`, `end_ms`, `speaker_label` |
| Deep link | opening the session at `start_ms` lands playback at the matched utterance |
| Casey: `q=tamarind&types=transcripts` | zero hits — transcripts are the richest corpus and the most damaging leak |
| Alex | sees both relationships' transcript hits; Globex transcripts never |

## 11. Scenario J: members search (PR 3+)

| Probe | Expected |
|---|---|
| Casey: `q=<Morgan's name>&types=users` | zero hits, silently dropped |
| Alex: `q=<Morgan's name>&types=users` | Morgan found (Acme member) |
| Alex: `q=<Pat's name>&types=users` | zero hits (Globex member) |
| Alex: member-with-no-relationship by name | found — membership, not participation, is the criterion for members search |
| Alex: any found user's `organization_ids` | contains **only Acme** — even if the user is also in Globex, foreign memberships must not leak |
| Sam: `q=<Pat's name>&types=users` | found, with full `organization_ids` |

## 12. Scenario K: MCP `search` tool parity (PR 4+)

The MCP tool calls the same domain function, so the leak surface should be
identical — verify it is, because the identity arrives via PAT instead of a
cookie.

Using Casey's PAT against `POST /mcp` (`search` tool):

| Probe | Expected |
|---|---|
| `keyword: "kumquat"` | R1 hits; no `score` field, no `<mark>` markers, each session-anchored hit carries a `session_url` |
| `keyword: "tamarind"` | zero hits |
| `types: ["users"]` | dropped/absent — users and organizations are not MCP-searchable in MVP |
| limit | default 10, values above 25 clamped |
| Robin's PAT after the Scenario G removal | `kumquat` → zero hits — PAT identity honors membership revocation too |

## 13. Notes caveat (until PR 5)

Notes are **not searched at all** before PR 5: note content lives only in
Tiptap (the backend never reads it back), so there is no note corpus to
index yet. Until the `docs-collab-server` projection lands:

| Check | Expected |
|---|---|
| `types=notes` alone | 200, zero hits — `notes` is a known token, never a 400 |
| `q=kumquat` (any types) | **no** hit of `"type": "note"` anywhere |
| Text typed into an R1 note in the collaborative editor | never matches |

Any `note`-typed hit before PR 5 means a searcher is reading the legacy
`notes` table — pre-Tiptap stale data that must not surface. Once PR 5
lands, re-run the leak scenarios (D, F, G, H) with note content included:
a hit on another relationship's note content is a leak like any other.

## 14. Failure triage

| Symptom | Likely cause |
|---|---|
| Foreign sentinel returns hits | a searcher's scope join is missing or wrong — treat as a release blocker |
| 403 instead of silent drop | type gating implemented at the wrong layer (middleware/handler instead of searcher selection) |
| Hits but empty/garbled snippets | `ts_headline` running over a different expression than the index/query |
| Correct hits but sequential-scan slowness | query expression drifted from the index expression (see the shared-constants rule in the implementation plan) |
| Soft-deleted topic or archived org appears | searcher missing its `deleted_at`/`archived_at` predicate |
