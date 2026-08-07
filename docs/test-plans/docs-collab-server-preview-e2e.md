# Preview end-to-end test plan: docs-collab-server integration

Manual validation of the self-hosted `docs-collab-server` and its integration
across the backend (`refactor-platform-rs`) and frontend (`refactor-platform-fe`),
run against a **deployed PR-preview environment** (per-PR container stack on the
preview host). For the single-machine local variant, see the companion
[docs-collab-server-local-e2e.md](docs-collab-server-local-e2e.md).

This plan is written to be followed by a human tester or by a Claude Code
instance driving server-side verification. Each check states who does it
(browser vs. server-side), the exact command where relevant, and a PASS
criterion. Substitute `<N>` with the preview PR number (e.g. `371`) and
`<preview-host>` with the SSH alias for the deployment host throughout.

## What is under test

Four collaborating processes in the `pr-<N>` stack, plus Postgres:

- **docs-collab-server** (`pr-<N>-docs-collab-1`, port 1234): Yjs/Hocuspocus-
  compatible collaboration server. Owns `refactor_platform.collab_documents`
  (`name TEXT PK`, `state BYTEA`, `updated_at`).
- **backend** (`pr-<N>-backend-1`, port 4000): mints the collab JWT and calls the
  collab server's REST `POST`/`DELETE /api/documents/{name}`.
- **frontend** (`pr-<N>-frontend-1`, port 3000): points its collaboration provider
  at the collab server via `NEXT_PUBLIC_DOCS_COLLAB_URL`.
- **postgres** (`pr-<N>-postgres-1`): shared DB.

Key facts the checks rely on:

- **Document naming**: `<org-slug>.<relationship-slug>.<uuid>-v0`
  (e.g. `refactor-group.jimrg-james.dafc6d19-...-v0`). The auth **scope** is the
  prefix up to the last `.` (`refactor-group.jimrg-james`), so a token is valid
  for one relationship's documents only.
- **Shared secrets** (mismatch is the most common failure): the collab server's
  `JWT_SIGNING_KEY` must equal the backend's `TIPTAP_JWT_SIGNING_KEY`, and its
  `MANAGEMENT_AUTH_KEY` must equal the backend's `TIPTAP_AUTH_KEY`.
- **Deferred document creation**: creating a session series does NOT create collab
  documents. A document is provisioned on demand the first time a session's notes
  are opened.
- **Persistence** is debounced write-behind: `PERSIST_DEBOUNCE_MS` (default 500)
  coalesces a burst into one store; a graceful shutdown flushes in-flight writes.

## Prerequisites

- The `pr-<N>` stack is deployed and healthy. Confirm (server-side):
  ```bash
  ssh <preview-host> 'docker ps --filter name=pr-<N> --format "{{.Names}}\t{{.Status}}\t{{.Image}}"'
  ```
  PASS = `backend`, `frontend`, `docs-collab`, `postgres` all `Up`. Note the image
  tags/digests if verifying a fresh deploy (a redeploy should show `Up <minutes>`,
  not `Up <days>`).
- Two browsers (use one normal + one incognito) so two users can be signed in at
  once. One tab per user per document (see the one-tab gotcha below).
- Test users (from the seed data). All passwords are the seed default:

  | User | Role | Relationship |
  |------|------|--------------|
  | `jim@refactorgroup.com` | Admin (Refactor Group) | coach of `james.hodapp` (Refactor Group) |
  | `james.hodapp@gmail.com` | Member (both orgs) | coachee of jim; coach of caleb (RG) |
  | `calebbourg2@gmail.com` | Member (both); Admin (Acme) | cross-org test user |

- Optional: direct Postgres access for a GUI client (e.g. Postico) via an SSH
  tunnel to the container's bridge IP (the DB port is not published on the host):
  ```bash
  ssh -N -L 5433:$(ssh <preview-host> "docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' pr-<N>-postgres-1"):5432 <preview-host>
  ```
  Then connect to `localhost:5433`, db `refactor`, user `refactor`, schema
  `refactor_platform`.

## How to verify server-side (what to observe)

The collab server exposes three objective signals; prefer them over trusting the
UI alone.

1. **Persistence**: the `collab_documents` row's byte length and `updated_at`
   change after edits.
   ```bash
   ssh <preview-host> 'docker exec pr-<N>-postgres-1 psql -U refactor -d refactor -A -F"|" \
     -c "SELECT name, octet_length(state) AS bytes, updated_at FROM refactor_platform.collab_documents ORDER BY updated_at DESC;"'
   ```
2. **Session-to-document mapping** (empty `collab_document_name` = no document yet):
   ```bash
   ssh <preview-host> 'docker exec pr-<N>-postgres-1 psql -U refactor -d refactor -A -F"|" \
     -c "SELECT id, date, collab_document_name FROM refactor_platform.coaching_sessions ORDER BY date DESC;"'
   ```
3. **Server logs** (startup, shutdown; auth decisions if the level is raised):
   ```bash
   ssh <preview-host> 'docker logs pr-<N>-docs-collab-1 --since 10m'
   ```

Backend document create/delete activity is visible in the backend log at INFO:
```bash
ssh <preview-host> 'docker logs pr-<N>-backend-1 --since 10m | grep -i "tiptap document"'
```

## Phase 1: Presence and awareness

1. **On-load presence, both directions.** User A opens a session; then User B opens
   the same session. Then repeat with B first, A second.
   PASS = each user's avatar turns green for the other **immediately on load**, with
   no cursor movement required, in both orderings.
2. **Reactive updates.** A moves the cursor or selects text.
   PASS = B sees A's cursor/selection move live.
3. **Leave.** A closes the tab.
   PASS = A's presence disappears for B within a couple of seconds.

## Phase 2: Real-time sync and CRDT convergence

4. **One-way live typing.** A types a paragraph. PASS = it appears for B as typed.
5. **Concurrent edits, different locations.** Both type in different paragraphs at
   once. PASS = no loss; both appear for both.
6. **Concurrent edits, same line.** Both type into the same sentence. PASS = the
   result is identical on both screens (interleaving is acceptable; divergence is
   not). Single-tester note: hard to stage alone; may be covered by the automated
   convergence test instead.
7. **Rich formatting.** Bold, italic, headings, lists, links. PASS = replicate
   correctly to the peer.

Server-side: `collab_documents` byte length grows and both screens match the
persisted state.

## Phase 3: Persistence and hydration

8. **Reload survives.** A edits, waits ~2s (past the debounce), reloads.
   PASS = content intact.
9. **Both clients gone.** Both close all tabs, wait, reopen. PASS = content intact.
10. **Process restart (hydrate from storage).** After an edit and a few seconds,
    restart the collab process so its in-memory state is wiped:
    ```bash
    ssh <preview-host> 'docker restart pr-<N>-docs-collab-1'
    ```
    Reload the editor. PASS = content is restored (it can only have come from the
    `collab_documents` BYTEA snapshot). Confirm the row byte counts are unchanged
    across the restart.

## Phase 4: Auth and access control

11. **New-session create path (REST create).** As `jim@refactorgroup.com`, create a
    new single coaching session in Refactor Group.
    PASS = a new `collab_documents` row appears, the session's `collab_document_name`
    is populated, and the backend log shows `Attempting to create Tiptap document
    with name: ...`. Open the notes and type; confirm it syncs and persists.
12. **Cross-org isolation.** As `calebbourg2@gmail.com`, switch the active
    organization to Acme, then attempt to open a Refactor Group session's notes
    (e.g. via a stale URL). PASS = access is denied (the frontend blocks with a
    "You don't have access to this organization" style message, and no valid collab
    token is issued for the out-of-scope document).
13. **Token scope (server-side / automated).** A token scoped to relationship A must
    not open relationship B's document. This is enforced by the collab server's
    authenticator and is covered by the frozen unit suite (see Automated coverage).
    Optional live check: sustained normal use should never log `PermissionDenied`.

## Phase 5: Resilience and edge cases

14. **Offline reconnect merge.** While typing as one user, disable that browser's
    network for ~10s, keep typing offline, then re-enable it.
    PASS = the offline edits propagate to the peer after reconnect (this exercises
    the server's SyncStep1-on-join handshake; without it, offline edits never merge).
15. **Rapid burst / large paste.** Paste several paragraphs at once, or hold a key.
    PASS = nothing is dropped on the peer side; the persisted state matches.
16. **Delete cleanup (REST delete).** Delete a session created in step 11.
    PASS = its `collab_documents` row is removed (re-run the persistence query and
    confirm the name is gone). Delete while idle: a live in-memory copy can re-write
    the row if someone is actively editing at delete time (see Known behaviors).

## Phase 6: Coaching session series integration (deferred creation)

17. **Series create.** As the relationship's **coach** (only the coach may create a
    series; e.g. `jim@refactorgroup.com` on `jimrg-james`), create a recurring
    series (e.g. 4 weekly sessions).
    PASS = the request succeeds. It targets `POST /coaching_session_series` (the
    renamed endpoint; the removed `POST /coaching_sessions/recurring` returns a
    misleading 400 "UUID parsing failed" if the frontend is stale, see
    Troubleshooting).
18. **No eager documents.** Immediately after creation (before opening any session):
    PASS = N new `coaching_sessions` rows exist, all with empty `collab_document_name`,
    and there are **zero** new `collab_documents` rows. No collab REST calls fire.
19. **On-demand creation.** Open one of the series sessions' notes.
    PASS = that session's `collab_document_name` becomes populated and a new
    `collab_documents` row materializes at open time.

## Phase 7: Graceful shutdown (SIGTERM flush)

20. **SIGTERM handling.** The collab server must flush in-flight (debounced) writes
    on `docker stop`/`restart`/`compose down`, which send SIGTERM.
    ```bash
    ssh <preview-host> '/usr/bin/time -v docker stop pr-<N>-docs-collab-1 2>&1 | grep -i "Elapsed"; \
             docker logs pr-<N>-docs-collab-1 --tail 5'
    ssh <preview-host> 'docker start pr-<N>-docs-collab-1'
    ```
    PASS = the stop returns in well under a second (not the ~10s SIGKILL grace
    period), and the logs show `SIGTERM received; initiating graceful shutdown`
    followed by `docs-collab-server stopped` (the latter is logged only after
    `flush_all()` completes). Confirm the container is `Up` and listening after
    `docker start`, and that document data is intact.

## Automated coverage (run before manual testing)

The manual phases are backed by the crate's test suite; run it first so manual
time is spent on integration, not regressions:
```bash
cargo test -p docs-collab-server
```
- `tests/auth.rs`: JWT scope enforcement (matching wildcard accepted; different
  org and different relationship rejected; expired / bad-signature / garbage
  rejected). Backs Phase 4 step 13.
- `tests/document_sync.rs`: two-client convergence and
  `persistence_survives_evict_and_reload`. Backs Phases 2 and 3.
- `src/registry_tests.rs`: `flush_all_flushes_every_live_document`, evict and
  concurrent-load invariants. Backs Phases 3 and 7.
- `src/document_tests.rs`: `sync_step1_reflects_current_state`, debounce
  coalescing, observer lifetime. Backs Phases 5 and 3.
- `tests/authz_e2e.rs`, `tests/e2e_provider.rs`, `tests/storage_pg.rs`: end-to-end
  WS and Postgres-backed checks. Marked `#[ignore]` pending an in-process server
  harness / a live Postgres; run explicitly when that harness exists.

## Troubleshooting (symptom to cause)

- Editor never connects / stuck "Preparing coaching notes"; no WebSocket in the
  Network tab: `NEXT_PUBLIC_DOCS_COLLAB_URL` not baked into the frontend build
  (it is inlined at build time, so it must be an `ARG`+`ENV` before `next build`),
  or the collab container is down.
- ~10s sync timeout, console auth error, collab log `PermissionDenied`:
  `JWT_SIGNING_KEY` does not equal the backend's `TIPTAP_JWT_SIGNING_KEY`.
- Session create errors, no document row provisioned: `MANAGEMENT_AUTH_KEY` does
  not equal the backend's `TIPTAP_AUTH_KEY`.
- Series create returns 400 "Invalid URL: UUID parsing failed: invalid character:
  found `r` at 1": the frontend is calling the removed `POST
  /coaching_sessions/recurring`; it must use `POST /coaching_session_series`. Sync
  the frontend branch with its `main`.
- Series create returns 403: the acting user is not the coach of the selected
  relationship (only the coach may create a series).
- Presence only turns green after a cursor move: the join-time awareness snapshot
  is missing (server must push existing peers' awareness on join).
- Offline edits never merge after reconnect: the server is not sending its
  SyncStep1 on join.

## Known behaviors (not bugs)

- **One tab per user per document.** Each browser tab is a distinct Yjs client, so
  opening the same user in multiple tabs of the same document fragments presence.
- **Idle reconnect every ~30s.** A connection idle past the provider's
  `messageReconnectTimeout` is closed and immediately reopened. Benign (seamless,
  no data loss) but presence can flicker when idle.
- **"All changes saved" may not resolve.** The server sends `SyncStatus` only after
  an `Update`, not after initial sync, so the provider's unsynced counter can stay
  nonzero. Not surfaced in the UI today.
- **Delete while editing.** Deleting a session whose document is actively being
  edited can let the live in-memory copy re-write the row after delete; delete when
  idle.

## Notes for a Claude Code instance driving this plan

- Read-only server-side inspection (`docker ps`, `docker logs`, `docker inspect`,
  `psql` SELECTs) and single-container `docker stop`/`start`/`restart` are safe to
  run directly.
- Destructive host operations (`docker rm -f`, `docker volume rm`, redeploys that
  wipe data) are blocked by the auto-mode classifier and must be requested from the
  human operator. Present the exact commands and wait for confirmation.
- Never materialize secrets (e.g. `POSTGRES_PASSWORD`) into command output; filter
  environment dumps to the non-secret fields you need.
- The two-browser interaction steps require a human; drive the server-side
  verification and report PASS/FAIL against each phase's criterion.
