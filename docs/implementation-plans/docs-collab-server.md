# Local self-hosted collaboration backend (Hocuspocus) — experiment

## Context

Refactor Coach's collaborative coaching notes are stored on **TipTap Cloud (Start
plan)**, which caps documents (~500) and prices by document count. With ~250 docs
today, +5–10/week solo, and beta orgs coming, document cost will not scale. The
goal is to replace the paid Cloud document backend with a **self-hosted
collaboration server** persisting Yjs state to our own Postgres, validated on the
local macOS dev laptop first. Editor stays TipTap; only the collaboration
*backend* changes.

Per the user's direction, the collaboration server is built as a **reusable,
idiomatic Rust Axum crate** (not a Node service). It reimplements the **Hocuspocus
wire protocol** so the existing `@hocuspocus/provider` client connects unchanged,
and is designed to grow into a general-purpose, shareable self-hosted replacement
for the parts of TipTap Cloud we rely on (websocket collab today; document REST,
metrics, webhooks later).

### Key facts verified in-repo / from source

1. **Frontend is already a Hocuspocus client.** It uses `TiptapCollabProvider`
   from `@hocuspocus/provider` (installed **v2.15.3**), a thin subclass of
   `HocuspocusProvider`. Verified in the installed source: its websocket sets
   `url = configuration.baseUrl ?? wss://<appId>.collab.tiptap.cloud` (line 2838)
   and connects to that URL with **no path appended** — the document name is sent
   **in-band**. Two ways to retarget locally: (a) keep `TiptapCollabProvider` and
   pass `baseUrl`, or (b) use base `HocuspocusProvider({ url })`. Small change.
2. **The app backend never touches the websocket, and stays unchanged.** It only
   (a) mints an HS256 JWT (`domain/src/jwt/mod.rs::generate_collab_token`) and
   (b) calls a REST management API to pre-create/delete doc shells
   (`domain/src/gateway/tiptap.rs`: `POST`/`DELETE /api/documents/{name}?format=json`).
   **Both stay used as-is — no Rust changes in the app crates.** Our server
   implements that REST surface; only the `tiptap_*` *env values* move.
3. **Tenancy is already encoded** in the doc name
   `{org_slug}.{relationship_slug}.{uuid}-v0` and the JWT claim
   `allowedDocumentNames = {org_slug}.{relationship_slug}.*`
   (`domain/src/jwt/mod.rs:64-67`). Our authenticator re-enforces the same wildcard.
4. **Rust ecosystem is ready, and the protocol reuse is deep** (read from
   `yrs 0.26.0` source — `src/sync/protocol.rs`, `src/updates/{encoder,decoder}.rs`):
   - `yrs::sync` bundles the y-sync protocol: `Message`
     (`Sync=0, Awareness=1, Auth=2, AwarenessQuery=3, Custom(u8,..)`), `SyncMessage`
     (`SyncStep1=0, SyncStep2=1, Update=2`), the `Protocol`/`DefaultProtocol`
     handlers, `Awareness`, `AwarenessUpdate`, `StateVector`, `MessageReader`, and
     `MSG_*` tag constants. **Tags 0–3 are byte-identical to Hocuspocus's outer
     types 0–3.**
   - lib0 codec is exposed via `yrs::updates::{encoder::EncoderV1, decoder::DecoderV1}`
     and the `yrs::encoding::{read::Read, write::Write}` traits
     (`read_string`/`write_string`, `read_var`/`write_var`, `read_buf`/`write_buf`)
     — the **same primitives the JS client uses**, so byte-compatibility is free.
   - `yrs::sync` doc-comment: *"A message does not include information about the room
     name. This must be handled by the upper layer protocol!"* — **Hocuspocus IS
     that layer**: it adds a leading `[varString documentName]`, three extra types
     (`Stateless=5, CLOSE=7, SyncStatus=8`), and a token sub-protocol inside Auth
     whose payload differs from `yrs::sync::Message::Auth` (so we model Auth ourselves).
   - `axum 0.7.7`, `jsonwebtoken 10` (aws_lc_rs — same crate the app signs with),
     `sqlx`, `tokio` are already in the workspace lockfile.

   **Reuse decision (build on generic `yrs 0.26`, do NOT depend on `yrs-warp`):**
   `yrs-warp 0.9.0` pins **`yrs 0.24`** + **`warp 0.3`** (verified from its
   crates.io `Cargo.toml`); `0.24`≠`0.26` (semver) would compile two incompatible
   `yrs` crates, and its codec is hardcoded to bare y-sync framing + one-doc-per-
   group. We reuse the *generic `yrs` + `yrs::sync`* (CRDT, sync protocol,
   awareness) and treat `yrs-warp/src/broadcast.rs` as a ~60-line **reference
   pattern, not a dependency**.

### Decisions (defaults chosen; tell me to flip any)
- **Doc import: START FRESH.** Prove the round-trip with new/empty docs; zero work
  on importing existing Cloud docs. The Cloud→Yjs import is a **separate phase only
  if the core experiment succeeds**.
- **App-backend Rust crates: UNCHANGED.** Only config/env values change.
- **Test-first, frozen tests.** Write the full suite *before* implementation —
  integration tests in their own `tests/` dir, unit tests alongside their modules —
  then physically freeze the `tests/` files with `chmod a-w` during implementation.
  Remaining failures are triaged at the end (after `chmod +w`) to tell a real
  implementation flaw from a broken test (see Testing strategy).
- **Crate: `sqlx` not SeaORM** for runtime persistence (see Database).
- **Crate placement: new workspace member, excluded from `default-members`** (like
  `testing-tools`), sharing the workspace lockfile/`target`, with **no dependency on
  app crates** so it extracts to its own published repo later. **Working name
  `docs-collab-server`** — purpose-conveying and trademark-free (avoids
  "TipTap"/"Hocuspocus"); alt `notes-collab-server`. Final/public name still open.

## The Hocuspocus wire protocol (recovered from the installed client — authoritative)

Every WebSocket binary message is lib0-encoded:
```
[varString documentName][varUint MessageType][payload...]
```
Document name **in-band** (first field) → multi-doc multiplexing on one socket.
Outer `MessageType`: `Sync=0, Awareness=1, Auth=2, QueryAwareness=3, Stateless=5,
CLOSE=7, SyncStatus=8`.

- **Sync (0)**: `[varUint step][...]` — `SyncStep1=0 → [varUint8Array stateVector]`,
  `SyncStep2=1 → [varUint8Array update]`, `Update=2 → [varUint8Array update]`.
- **Awareness (1)**: `[varUint8Array awarenessUpdate]`.
- **Auth (2)**: `[varUint AuthMessageType][...]` — `Token=0 → [varString token]`
  (client→server), `PermissionDenied=1 → [varString reason]`,
  `Authenticated=2 → [varString scope]` (server→client).
- **QueryAwareness (3)**: name+type only → server replies Awareness for all states.
- **Stateless (5)**: `[varString payload]`. **SyncStatus (8)**: `[varInt 0|1]`.

**SyncStatus semantics (verified from provider).** The client's `synced` flag — the
one that clears the frontend's 10s `SYNC_TIMEOUT_MS` — is set on receiving
**`SyncStep2`**, NOT on `SyncStatus`. `SyncStatus(1)` is a per-update
**acknowledgment to the sending client** that decrements its `unsyncedChanges`
counter (`applySyncStatusMessage` → `decrementUnsyncedChanges`), driving the "all
changes saved" state. Decision: the server sends `SyncStatus(true)` **to the
originating client only** after it has applied+persisted that client's `Update`
(ack semantics; not broadcast). Required for the "saved" indicator, not for initial
sync — so the verification pass criterion (no 10s timeout) hinges on replying
`SyncStep2`, which the handshake already covers.

**Handshake (verified from provider `onOpen`, lines 2655-2660):** the client
**pipelines** on open — `Auth/Token(token)` *then immediately* `SyncStep1`
(+ `Awareness`), without waiting. Server, in arrival order:
1. `Auth/Token` → verify JWT + doc-name authz → `Auth/Authenticated(scope)`, or
   close with `Unauthorized`/`Forbidden` close code + `Auth/PermissionDenied(reason)`.
2. queued `SyncStep1` → reply `SyncStep2` + own `SyncStep1`; client sets `synced`.
3. `Update`/`Awareness` flow both ways and fan out to peers.

Implication: the server must **not** apply/echo the pipelined sync frame until the
connection is authenticated (verify first, even though frames arrive together).

## Architecture (local)

```
Next.js (:3000, host) ──ws──▶ docs-collab-server (Rust/axum) ──┐
       │  GET /jwt/generate_collab_token   ws "/"               │ Storage (sqlx)
       ▼                                   REST /api/documents/:name
  app backend (:4000, host) ──REST(create/delete)───────────────┤
                                                                 ▼
                            Postgres (:5432, Postgres.app, host)
                            table: collab_documents (name PK, state BYTEA, updated_at)
```
docs-collab-server runs bare-metal on the host (Rust binary; no Docker). WS at `/`
(provider connects to the configured URL root, no path), REST at `/api/documents/:name`.

## New crate: `docs-collab-server/` (Rust, axum) — module layout

Binary+lib crate depending only on third-party crates, **pinned to versions already
in the workspace lockfile** to avoid duplicate copies:
- `axum = "0.7"` (lockfile has **0.7.9** — do NOT write `0.7.7`; the plan said that
  earlier and it's wrong).
- `sqlx = { version = "0.8", features = ["postgres", "time", "runtime-tokio"] }` —
  **must declare `postgres` explicitly**: the app crates get the PG driver
  transitively via sea-orm's `sqlx-postgres`, but this crate doesn't use sea-orm, so
  without `postgres` the `PgPool`/query types don't exist. Resolves to 0.8.6.
- `dashmap = "6"` (lockfile has both 5.5.3 and 6.2.1 — pin `6` to reuse 6.2.1, not a
  third copy).
- `yrs = "0.26"` (`sync` + lib0 codec), `jsonwebtoken = "10"`, plus `tokio`,
  `tokio-stream` (for `BroadcastStream`), `futures` (`FuturesUnordered`), `clap`,
  `bytes`, `thiserror`, `tracing`.

**No `yrs-warp`** (pins incompatible `yrs 0.24` + drags in `warp`); its
`broadcast.rs` is a copy-reference only.

- **`protocol.rs`** — the **only genuinely new protocol code**: a pure, I/O-free
  Hocuspocus framing layer over `yrs`'s lib0 codec. Design goal: model the wire
  protocol as a total Rust type (parse-don't-validate) so every valid frame is
  representable and malformed input is a typed error. Reuse `yrs` payload types so
  we never hand-roll CRDT encoding:
  ```rust
  pub struct Frame { pub name: String, pub body: Body }

  pub enum Body {
      // y-protocol payloads (outer tags 0/1/3) — reuse yrs types verbatim
      SyncStep1(StateVector),
      SyncStep2(Vec<u8>),          // update bytes
      Update(Vec<u8>),
      Awareness(AwarenessUpdate),
      AwarenessQuery,
      // Hocuspocus-only (outer tags 2/5/7/8) — modeled by us
      AuthToken(String),           // client→server (tag 2, AuthMessageType::Token=0)
      Authenticated(String),       // server→client (tag 2, =2) — `scope`
      PermissionDenied(String),    // server→client (tag 2, =1)
      Stateless(String),           // tag 5
      SyncStatus(bool),            // tag 8
      Close,                       // tag 7
  }
  ```
  Codec via `EncoderV1`/`DecoderV1` + the `Read`/`Write` traits
  (`read_string`/`write_string`, `read_var`/`write_var`, `read_buf`/`write_buf`) —
  the same lib0 primitives the JS client uses. `Frame::decode(&[u8])` reads `name`,
  then the outer tag (`read_var::<u8>`), then dispatches: tags `0/1/3` decode
  payloads via `SyncMessage`/`AwarenessUpdate`'s own `Decode` impls; tag `2` parses
  the `AuthMessageType` sub-tag (we cannot delegate to `yrs::sync::Message::Auth`,
  whose payload differs — verified); tags `5/7/8` parse directly.
  `Frame::encode(&self) -> Vec<u8>` is the symmetric inverse. Errors: a `thiserror`
  `ProtocolError` (`UnknownTag(u8)`, `Truncated`, `Utf8`, …). Reuse
  `MSG_SYNC_STEP_1/2/UPDATE` from `yrs::sync::protocol`. No async, no sockets.
- **`document.rs`** — per-document shared state: `Arc<RwLock<yrs::sync::Awareness>>`
  (Awareness owns the `Doc`), loaded from `Storage` on first open, plus a
  `tokio::sync::broadcast::Sender<Bytes>`. A `doc.observe_update_v1` subscription
  pushes each local update onto the channel (pattern from `yrs-warp/src/broadcast.rs`).
  **The returned `yrs::Subscription` MUST be stored as a named field** (e.g.
  `_update_sub: yrs::Subscription`): the callback unregisters when the handle drops,
  so failing to hold it makes updates fire once then silently stop. Sync/awareness
  *handling* delegates to `yrs::sync::DefaultProtocol` (no reimplemented merge logic).
  Persistence: debounced write-behind on change (coalesce a burst into one
  `Storage::store`) + final flush on last-disconnect.
- **`registry.rs`** — `DocumentRegistry` over `DashMap<String, Arc<Document>>`:
  `get_or_load(name)` is the only entry; `Arc`-refcount + idle timer evicts (after
  flush) when the last connection leaves. Frames route by their in-band name, so one
  socket multiplexes many docs.
- **`auth.rs`** — `Authenticator` trait: `authenticate(token, doc_name) -> Result<Scope>`.
  Default impl verifies HS256 via `jsonwebtoken` and **MUST set
  `validation.validate_aud = false`** (proven: jsonwebtoken 10 defaults
  `validate_aud=true`; a token carrying `aud=tiptap_app_id` with no configured
  audience → `InvalidAudience`). `exp` validated by default (app sets 24h out; OK);
  `nbf` not validated (and the claim is the misspelled `ndf`, an ignored custom
  field). Then enforce the `allowedDocumentNames` wildcard (`{org}.{rel}.*` prefix
  match). Pluggable, but drop-in for the current token shape.
- **`storage.rs`** — `Storage` trait: `fetch`/`store`/`delete`. `PostgresStorage`
  via `sqlx` against `collab_documents`; `MemoryStorage` for tests. **Uses the
  runtime `sqlx::query()` API, not the compile-time `sqlx::query!()` macros** — so
  the crate and its test suite compile with no live DB and no committed `.sqlx/`
  offline snapshot (keeps it standalone/publishable; the trivial single-table SQL
  doesn't need macro checking). **Startup bootstrap runs `CREATE SCHEMA IF NOT
  EXISTS <schema>` *then* `CREATE TABLE IF NOT EXISTS`** — the schema step is
  required because a fresh DB (e.g. the test harness) won't have `refactor_platform`
  yet. Schema name is a config field (default `refactor_platform`).
- **`ws.rs`** — axum `WebSocketUpgrade` at `/`. **Concurrency model (fully async
  tokio, one connection = one actor):** on upgrade, `StreamExt::split` →
  `(SplitSink, SplitStream)`; spawn one task per connection running `tokio::select!`
  over three sources — (1) **inbound** client frames → decode → auth-gate → apply via
  registry → reply (`SyncStep2` to a `SyncStep1`; `SyncStatus(true)` to the sender
  after applying+persisting its `Update`); (2) **broadcast** peer updates → this
  sink; (3) **shutdown** (server signal / client close). Outbound writes funnel
  through a per-connection `mpsc::Sender<Bytes>` so the sink has a single writer.

  **Dynamic multi-doc fan-in (concern):** `tokio::select!` is a static macro and
  cannot select over a runtime-sized set of `broadcast::Receiver`s, yet one socket
  may join many docs. Use a **`FuturesUnordered<BroadcastStream<Bytes>>`** (one
  `tokio_stream::wrappers::BroadcastStream` per joined doc) polled in a *single*
  `select!` arm; joining/leaving a doc pushes/removes a stream. (Alt: a dedicated
  per-connection fan-in task feeding the same `mpsc`.) The Phase-2 multi-doc
  integration test asserts this works.

  **No echo to sender (concern):** each connection has a `ConnectionId`; updates it
  applies are published to the doc's `broadcast::Sender` **tagged with that id**, and
  every consumer **skips frames carrying its own id** — otherwise the originator
  re-receives its own update (double-delivery). The sender's own ack is the separate
  `SyncStatus(true)`; peer fan-out is the broadcast.

  Hundreds of sockets = hundreds of cheap tasks over a shared `Arc<DocumentRegistry>`;
  per-doc state shared via `Arc`, mutated under a short `RwLock` write only when
  applying an update. Bounded channels give backpressure; a lagging `BroadcastStream`
  (`Lagged`) forces a resync rather than dropping the connection.
- **`rest.rs`** — axum `POST`/`DELETE /api/documents/:name`. **Auth compares the
  `Authorization` header *verbatim* to the management key — NO `Bearer` prefix**
  (verified: `gateway/tiptap.rs:26-36` sets the raw key directly as the header
  value). Accepts the `?format=json` query the app appends; `POST` upserts an empty
  row (idempotent, returns 2xx/409 like Cloud), `DELETE` removes it (2xx/404).
  Writes through `Storage`. Extension point for future Cloud-parity features (e.g.
  `GET /api/documents/:name?format=json` for metrics).
- **`config.rs`** — **emulates the existing `service/src/config.rs` idioms** (clap
  `derive` with `#[arg(long, env)]`, private fields + accessor methods, a `from_args`
  test constructor), but self-contained (no dependency on `service`). Fields:
  `--database-url`, `--jwt-signing-key`, `--management-auth-key`, `--bind-addr`
  (default `0.0.0.0:1234`), idle/persist-debounce timeouts.
- **`main.rs`** — builds the axum `Router` (WS `/` + REST `/api/documents/:name`),
  shares `AppState { registry, storage, authenticator, config }`. Installs a
  `tokio::signal::ctrl_c()` handler that signals all connection tasks and **awaits
  their final flush** before exit, so a deliberate shutdown doesn't lose
  debounced-but-unwritten updates (without it, killing mid-burst drops pending
  writes — acceptable for a crash, not for an orderly stop).

Add `docs-collab-server` to `[workspace].members` but NOT `default-members` in the
root `Cargo.toml` (mirrors the `testing-tools` exclusion).

## Database

Local DB is named **`refactor`** (role `refactor`, `localhost:5432`); the app's
tables live in the **`refactor_platform`** schema. The crate bootstraps both schema
and table at startup:
```sql
CREATE SCHEMA IF NOT EXISTS refactor_platform;          -- required on a fresh DB
CREATE TABLE IF NOT EXISTS refactor_platform.collab_documents (
  name       TEXT PRIMARY KEY,
  state      BYTEA NOT NULL,        -- Yjs binary document state
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Table name — `collab_documents`, not `yjs_documents`.** The `yjs_` prefix leaks
the storage *implementation* (CRDT format) into the schema; the table's role is "a
collaborative document's persisted state," which `collab_` conveys without coupling
to Yjs (captured in the `state` column comment). It also reads consistently with the
crate and won't be confused with an app-domain concept in the shared schema.

**sqlx vs SeaORM — a real (if modest) difference, decided for sqlx.** SeaORM *for
this crate* is a real cost, not imaginary: it pulls the app's heavier ORM + entity
codegen into a crate needing exactly one table with blob upsert/fetch/delete, and
couples a reusable crate to SeaORM. `sqlx` gives compile-time-checked queries, a
tiny surface, and portability — so the crate uses `sqlx` for all runtime CRUD and
creates its table at startup. The concerns are distinct: if we later ship to prod,
add a SeaORM migration in the *app's* `migration/` crate purely to own DDL/ownership
(`schema-qualify + OWNER TO refactor` per project rules), while docs-collab-server
still reads/writes via `sqlx`. Postgres via **Postgres.app only** (no brew-install).

## App backend (Rust) — config/env only, ZERO code changes

Point the existing four `tiptap_*` settings (`service/src/config.rs:241-255`) at the
local docs-collab-server:
- `--tiptap-url=http://localhost:1234` (REST base; gateway appends
  `/api/documents/{name}?format=json`)
- `--tiptap-auth-key=<dev-management-key>` (REST header check)
- `--tiptap-jwt-signing-key=<dev-shared-secret>` (MUST equal the crate's
  `--jwt-signing-key`)
- `--tiptap-app-id=<anything>` (only the JWT `aud`; server ignores it)

No `docker-compose`/deploy-workflow changes (production passthroughs, out of scope
while local-only).

## Frontend (Next.js) — one real change + env

1. **`src/components/ui/coaching-sessions/editor-cache-context.tsx`** (~line 288).
   Two equivalent options:
   - **(a) Minimal** — keep `TiptapCollabProvider`, pass
     `baseUrl: siteConfig.env.docsCollabUrl` (verified override at line 2838), drop
     `appId`. Smallest diff, no type changes.
   - **(b) Cleaner** — `new HocuspocusProvider({ url: siteConfig.env.docsCollabUrl,
     name: jwt.sub, token: jwt.token, document: doc })` (base class, same package);
     pairs with the type-alias swap.
   Either way, all awareness/presence/sync/timeout logic stays.
2. **Type aliases**: switch `TiptapCollabProvider` (used only as a *type*) to
   `HocuspocusProvider` in `editor-cache-context.tsx`,
   `coaching-notes/extensions.tsx`, `coaching-notes/connection-status.tsx`.
3. **Config**: add `docsCollabUrl: process.env.NEXT_PUBLIC_DOCS_COLLAB_URL` to
   `site.config.ts`; set `NEXT_PUBLIC_DOCS_COLLAB_URL=ws://localhost:1234` in
   `.env.local`. Leave `tiptapAppId` in place so flipping back is trivial.
4. `useCollaborationToken` and the `jwt.sub`-as-doc-name flow are unchanged.

## Testing strategy (test-first, frozen)

**Principle:** write as many tests as possible **before** implementation, then
**freeze them** — no edits during implementation. At the end, any remaining failure
is triaged as either a real implementation flaw or a genuinely wrong test, never
silently massaged to pass. This guards against tests being bent to fit the code.

**File layout + mechanical freeze:**
- **Integration tests live in their own `tests/` directory** (separate files, each
  compiled as its own crate against the lib's public API) — e.g.
  `tests/protocol_conformance.rs`, `tests/auth.rs`, `tests/storage_pg.rs`,
  `tests/document_sync.rs`, `tests/e2e_provider.rs`, plus committed
  `tests/fixtures/`. These are the **authoritative, bias-resistant gate**.
- **Unit tests live alongside their module** as idiomatic `#[cfg(test)] mod tests`
  blocks — fast inner-loop checks for the implementer.
- **Freeze mechanism:** after the test suite is authored (Phase 2), run
  `chmod a-w` on the `tests/` files and `tests/fixtures/` so they're physically
  read-only during implementation; changing one requires a deliberate `chmod +w`,
  making any test edit conscious and visible (and avoided per the principle). The
  freeze is lifted only at the final triage phase. (In-file unit tests share their
  module file so they can't be chmod-frozen; the frozen `tests/` suite is what
  enforces the no-bias guarantee, so the highest-value conformance tests go there.)
- **API implication:** because `tests/` integration tests see only the public API,
  `protocol.rs` (and the `Storage`/`Authenticator` traits, `Document`/`Registry`
  entry points) expose a clean `pub` surface. Genuinely internal white-box checks
  (e.g. eviction internals) stay as in-file unit tests.

Tests authored up front, per layer (→ where each lives):
- **`protocol.rs` (pure, highest value):**
  - **Ground-truth fixtures** — capture real byte sequences emitted by the installed
    `@hocuspocus/provider` (a tiny Node harness that constructs each
    Outgoing*Message and dumps the `Uint8Array`), committed under
    `tests/fixtures/`. Assert `Frame::decode(fixture)` yields the expected `Body`,
    and `Frame::encode(body)` reproduces the fixture **byte-for-byte**.
  - Property/round-trip: `decode(encode(frame)) == frame` for arbitrary frames
    (`proptest`), including the awkward cases (empty awareness, large updates,
    unicode doc names, the Auth sub-tags, Stateless/Close/SyncStatus).
  - Negative: truncated buffers, unknown outer tags, bad utf8 → typed `ProtocolError`.
- **`auth.rs`:** a token minted with the *same* claim shape + secret the app uses
  (reuse `jsonwebtoken` to mint in-test) verifies; assert `validate_aud=false` is
  required (a token with `aud` fails when it's true); wildcard accepts
  `org.rel.<uuid>-v0` and rejects a different `org.rel`; expired token rejected.
- **`storage.rs`:** `MemoryStorage` unit tests; `PostgresStorage` integration tests
  (gated on a `DATABASE_URL`, `#[ignore]` by default) for store/fetch/delete +
  upsert idempotency.
- **`document.rs`/`registry.rs`:** two in-process `yrs::Doc`s synced through our
  `Document` converge; awareness add/remove; idle eviction flushes then drops;
  persistence survives an evict→reload cycle.
- **End-to-end integration (the true protocol test):** a Rust integration test that
  boots the server on an ephemeral port with `MemoryStorage` and drives it with the
  **real `@hocuspocus/provider`** from Node (spawned harness) — two providers on one
  doc converge; reconnect merges; a wrong-scope token is rejected. (If spawning Node
  in CI is undesirable, a `tokio-tungstenite` client replaying the captured fixtures
  is the fallback — still exercises the real wire bytes.)

These tests are written in the early phases and then left untouched.

## Execution protocol (how we implement)

- **One phase at a time.** Implement a single phase, get it building/passing what it
  can, **commit anything that needs committing** (conventional, scoped commits), then
  stop. Don't run ahead into the next phase.
- **Per-phase handoff for a fresh Claude.** At the end of each phase, produce a
  self-contained handoff for a brand-new Claude with **no prior context**: include
  only what that next phase needs — the relevant slice of this plan, the exact files/
  APIs/decisions in play, how to build/test, and the current state — and nothing
  more. (The plan's per-module specs and the "proven facts" are the source to quote
  from.)
- **The plan lives in the repo.** As the *first* implementation step (Phase 1), copy
  this plan to **`docs/implementation-plans/docs-collab-server.md`** in the backend
  repo. Treat that copy as the **living source of truth**: whenever details change
  during implementation, update it in the same commit so it stays accurate.
- **Branch off `main` first, both repos.** Before any new work, create a fresh
  feature branch off `main` in **both** the backend (`refactor-platform-rs`) and the
  frontend (`refactor-platform-fe`) — per project convention, never build on the
  current branch. (Backend's current branch is `feat/288-tiptap-document-metrics`;
  start clean from `main`.)

## Implementation phases

0. **Branches + safety net.** Create the new feature branch off `main` in both
   `refactor-platform-rs` and `refactor-platform-fe`. Snapshot the local `refactor`
   DB via the `snapshot-local-db` skill and verify the restore round-trips.
1. **Crate scaffold** + workspace wiring (member, excluded from default-members);
   **save this plan to `docs/implementation-plans/docs-collab-server.md`** as the
   first step; `config.rs` skeleton; minimal `pub` API stubs (`todo!()`) so test
   files compile; correct dep versions (sqlx `postgres` feature, dashmap `6`, axum
   `0.7`). Commit.
2. **Write the frozen test suite**: in-file `#[cfg(test)]` unit stubs + the `tests/`
   integration files + the Node fixture-capture harness + committed
   `tests/fixtures/`. Confirm the suite compiles and fails against the stubs, then
   **`chmod a-w tests/ tests/fixtures/`** to physically freeze it. From here the
   `tests/` files are not edited.
3. **`protocol.rs`** until its unit/property/fixture tests pass.
4. **`storage.rs`** (`PostgresStorage` + `MemoryStorage`) + table bootstrap.
5. **`document.rs`/`registry.rs`** (yrs lifecycle, broadcast, persist, evict).
6. **`auth.rs`** (HS256 verify, `validate_aud=false`, wildcard).
7. **`ws.rs` + `rest.rs` + `main.rs`** — assemble; run the end-to-end test.
8. **Frontend swap** (provider + types + env).
9. **Run + wire**: start docs-collab-server, app backend with matching `--tiptap-*`,
   frontend with `NEXT_PUBLIC_DOCS_COLLAB_URL`; manual end-to-end pass.
10. **Triage frozen-test results** — only now `chmod +w` the `tests/` files; study any
    remaining failures; classify each as implementation flaw vs. broken test, and
    decide fixes deliberately.
11. **(Deferred) Import** of ~250 Cloud docs once the round-trip is proven.

## Verification (end-to-end, local)

Beyond the frozen automated suite above:
1. **Provisioning**: create a session in the UI → app backend logs the create →
   docs-collab-server `POST /api/documents/...` → row in `collab_documents`.
2. **Token + connect**: `GET /jwt/generate_collab_token?coaching_session_id=...`
   returns a JWT whose `sub` = doc name; provider authenticates + syncs (no 10s
   sync-timeout warning in the browser console).
3. **Real-time sync**: two browser windows on one session — edits and presence/
   cursors propagate.
4. **Persistence across restart**: edit, restart the server, reload — content
   survives.
5. **Offline/merge**: disconnect, edit, reconnect — Yjs CRDT merges.
6. **Authz**: a JWT scoped to relationship A is rejected (PermissionDenied) on a doc
   of relationship B.
7. **Delete**: delete the session → REST `DELETE` → row removed.
8. Dogfood against daily notes for ~a week before considering prod/beta.

## Explicitly out of scope
- Semantic search / embeddings / pgvector / reindex pipeline.
- docker-compose / deploy-workflow / production env wiring.
- SeaORM migration for `collab_documents` (until we decide to ship).
- The ~250-doc Cloud import (deferred phase).
- TipTap editor major upgrade / Hocuspocus v4 provider bump (clean follow-up; the
  Rust server targets the installed v2.15.x protocol, which is stable).

## Assumptions: proven vs. still to verify

**Proven from source / published metadata (not memory):**
- Hocuspocus wire format, message types, pipelined Auth+Sync handshake — installed
  `@hocuspocus/provider` **2.15.3** dist.
- `TiptapCollabProvider` extends `HocuspocusProvider`, accepts `baseUrl`, connects
  with no path append (doc name in-band) — installed source 2838/2858.
- App backend's only TipTap touchpoints are JWT mint + REST `POST`/`DELETE
  /api/documents/{name}?format=json`, accepting 2xx/409 on create, 2xx/404 on
  delete — `gateway/tiptap.rs`.
- `yrs 0.26.0` `sync` exports + lib0 `Read`/`Write` methods + `MSG_*` tags + that
  `Message::Auth` differs from Hocuspocus auth — read from the crate source.
- `yrs-warp 0.9.0` pins `yrs 0.24` + `warp 0.3` — reason we don't depend on it.
- `jsonwebtoken 10` rejects a token carrying `aud` unless `validate_aud=false` —
  drives a hard requirement in `auth.rs`.

**Still to verify at runtime/implementation (not assumable from docs):**
- Local Postgres confirmed reachable this session: Postgres.app **pg 17.10**, DB
  `refactor`, role `refactor`, password `password` (from `scripts/rebuild_db.sh` +
  the `sync-prod-db` skill), schema `refactor_platform` present. Connection:
  `postgres://refactor:password@localhost:5432/refactor`.
- `axum 0.7.7` `WebSocketUpgrade` delivers Hocuspocus binary frames intact (proven
  only when the round-trip runs / the e2e test passes).
- End-to-end CRDT interop with a real `@hocuspocus/provider` — Phase-2 fixtures +
  Phase-7 e2e + Phase-9 manual.
- Exact `yrs 0.26` method ergonomics (`Protocol` handler args,
  `Awareness::update_with_clients`) — pinned when scaffolding compiles.
- Phase-11 only: TipTap Cloud's REST export format for the doc import.

## Open questions
- **Final crate name** (esp. if published) — `docs-collab-server` working name;
  avoid trademarked names.
- **Phase-11 import**: confirm TipTap Cloud's REST export format
  (`GET /api/documents/{name}` — JSON vs Yjs binary) and the cleanest conversion to
  a Yjs binary state for seeding `collab_documents`.
