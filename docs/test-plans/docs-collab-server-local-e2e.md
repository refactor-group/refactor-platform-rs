# Local end-to-end runbook: self-hosted docs-collab-server

Manual verification that the unchanged frontend + app backend talk to the
self-hosted `docs-collab-server` instead of TipTap Cloud, on one macOS dev machine.
Run-and-observe; no code edits. (Phase 9 of the docs-collab-server build.)

## The one thing that breaks this: shared secrets must match

Three processes share two secrets. Get these wrong and it fails in confusing ways:

| docs-collab-server flag/env      | must EQUAL app-backend `.env` | why |
|----------------------------------|-------------------------------|-----|
| `JWT_SIGNING_KEY`                | `TIPTAP_JWT_SIGNING_KEY`      | the app mints the collab JWT (HS256) with this key; the collab server verifies with it. Mismatch → every WS auth is `PermissionDenied` → the editor shows the 10s sync timeout. |
| `MANAGEMENT_AUTH_KEY`            | `TIPTAP_AUTH_KEY`             | the app's REST create/delete sends `Authorization: <key>` verbatim; the collab server compares byte-for-byte. Mismatch → session create returns 401 → no document row is provisioned. |

The app also sends `aud = TIPTAP_APP_ID` in the token; the collab server ignores
`aud` (validates signature + `exp` + the `allowedDocumentNames` wildcard only), so
`TIPTAP_APP_ID` can stay whatever it is.

**Strategy used below: reuse the app's existing secrets** so only ONE app line
changes (`TIPTAP_URL`). The collab server is handed the app's existing
`TIPTAP_JWT_SIGNING_KEY` / `TIPTAP_AUTH_KEY` values at launch.

## Prereqs
- Postgres.app running; DB `refactor` reachable at `localhost:5432` (role `refactor`).
- Optional safety net: snapshot the DB first (`snapshot-local-db` skill) so you can
  restore after testing.
- Backend repo on branch `feat/docs-collab-server`; frontend repo on
  `feat/docs-collab-server`.
- Frontend `.env.local` already has `NEXT_PUBLIC_DOCS_COLLAB_URL="ws://localhost:1234"`.

## One app-backend `.env` change
In `refactor-platform-rs/.env`, repoint the REST base URL from Cloud to local:
```
# was: TIPTAP_URL=https://YOUR_APP_ID.collab.tiptap.cloud
TIPTAP_URL=http://localhost:1234
```
Leave `TIPTAP_AUTH_KEY`, `TIPTAP_JWT_SIGNING_KEY`, `TIPTAP_APP_ID` as-is. (If there
are duplicate `TIPTAP_URL` lines in `.env`, the last one wins — make sure the final
one is the local URL.)

## Start the three services (three terminals)

### Terminal 1 — docs-collab-server (port 1234)
Reads only process env (no `.env`), so pass values inline. This pulls the two shared
secrets straight from the app `.env` so they match by construction:
```bash
cd ~/Projects/refactor-coaching/refactor-platform-rs
set -a; source .env; set +a            # load app .env into this shell
JWT_SIGNING_KEY="$TIPTAP_JWT_SIGNING_KEY" \
MANAGEMENT_AUTH_KEY="$TIPTAP_AUTH_KEY" \
DATABASE_URL="$DATABASE_URL" \
DATABASE_SCHEMA=refactor_platform \
BIND_ADDR=127.0.0.1:1234 \
RUST_LOG=info,docs_collab_server=debug \
cargo run -p docs-collab-server
```
Expect: `docs-collab-server listening addr=127.0.0.1:1234`. (Schema/table bootstrap
logs are normal, including "already exists, skipping".)

### Terminal 2 — app backend (port 4000)
```bash
cd ~/Projects/refactor-coaching/refactor-platform-rs
RUST_LOG=info cargo run
```
(`docs-collab-server` is excluded from `default-members`, so plain `cargo run` builds
and runs the app backend, not the collab server.) Expect it to bind on
`BACKEND_PORT=4000` and connect to the DB. It loads `.env` via dotenvy
(`service/src/lib.rs`), so the `TIPTAP_URL=http://localhost:1234` change is picked up
on restart.

### Terminal 3 — frontend (port 3000)
```bash
cd ~/Projects/refactor-coaching/refactor-platform-fe
npm run dev
```
Open http://localhost:3000 and log in.

## Verification checklist

1. **Provisioning (REST create).** Create a coaching session in the UI. In Terminal 1
   watch for a `POST /api/documents/<org>.<rel>.<uuid>-v0` hit; confirm the row:
   ```bash
   psql "$DATABASE_URL" -c "select name, octet_length(state), updated_at from refactor_platform.collab_documents order by updated_at desc limit 5;"
   ```
   PASS = a new row for the session's document name (state ~2 bytes = empty seed).

2. **Token + connect (WS auth + initial sync).** Open that session's notes editor.
   In the browser devtools Network tab, the WS to `ws://localhost:1234/` should open
   and stay open; **no 10s "sync timeout" warning** in the console. Terminal 1 shows
   the auth + sync frames. PASS = editor is editable and "connected".

3. **Real-time sync.** Open the same session in a second browser window (or
   incognito, second login). Type in one; text appears in the other within a beat.
   Presence/cursor of the other user shows. PASS = both converge.
   **GOTCHA: one tab per user.** Each browser tab is a distinct Yjs `clientID`, so
   opening the *same* user in multiple tabs of the same doc fragments presence and
   makes it look one-directional. Test with exactly one tab per simulated user.

4. **Persistence across restart.** Type some notes, wait ~1s (debounce), confirm the
   row's `octet_length(state)` grew (re-run the psql query). Ctrl-C Terminal 1, watch
   for `docs-collab-server stopped` (graceful flush), restart it, reload the editor.
   PASS = your text is still there.

5. **Offline / CRDT merge.** With two windows open, stop Terminal 1, type different
   edits in each window, restart Terminal 1, let both reconnect. PASS = both edits
   merge (no clobber, no crash).

6. **Authz (wrong scope rejected).** Optional/manual: a token scoped to relationship
   A must not open a relationship-B document. Easiest check: confirm normal use never
   logs `PermissionDenied` in Terminal 1; a forced cross-scope attempt should.

7. **Delete (REST delete).** Delete the coaching session in the UI. Terminal 1 shows
   `DELETE /api/documents/<name>`; the row disappears:
   ```bash
   psql "$DATABASE_URL" -c "select count(*) from refactor_platform.collab_documents where name = '<that-name>';"
   ```
   PASS = 0. (Known limitation: if someone is actively editing that doc at delete
   time, the live in-memory copy can re-write the row; delete when idle. See the
   prod-hardening follow-ups.)

## Troubleshooting (symptom → cause)
- Editor shows ~10s sync timeout, console auth error, Terminal 1 logs
  `PermissionDenied` → `JWT_SIGNING_KEY` ≠ app `TIPTAP_JWT_SIGNING_KEY`.
- Creating a session 500s/errors; no document row; Terminal 1 logs a 401 on
  `POST /api/documents/...` → `MANAGEMENT_AUTH_KEY` ≠ app `TIPTAP_AUTH_KEY`.
- WS never connects / `ECONNREFUSED` → collab server not running, wrong port, or
  frontend `NEXT_PUBLIC_DOCS_COLLAB_URL` mismatched with `BIND_ADDR`.
- App backend still hits Cloud → `TIPTAP_URL` not actually changed (duplicate line
  later in `.env` overriding it), or app not restarted after the edit.

## Known behaviors (not bugs)
- **Idle reconnect every ~30s.** A connection that receives no messages for the
  provider's `messageReconnectTimeout` (30s default) is closed by the client and
  immediately reopened. Our server sends nothing during idle and excludes the sender
  from awareness fan-out, so a lone idle tab gets no traffic and recycles every ~33s.
  Benign (seamless reconnect, no data loss), but it makes presence flicker when idle.
  Confirmed via tcpdump on `lo0:1234`: single idle tab = exactly one connection at a
  time (no leak); it just closes+reopens. With N idle tabs the reconnects stagger, so
  it can look like "a new WebSocket every few seconds" in the Network tab.
  - Fix is deferred to the server-side follow-ups (do it correctly to spec, not via
    the client's `forceSyncInterval` — that re-arms the provider's `unsyncedChanges`
    counter each tick, which our server never clears, and WS ping/pong does not reset
    the provider's data-frame liveness timer). Tied to the `SyncStatus`-after-sync
    follow-up below.
- **`unsyncedChanges` / "all changes saved" never resolves.** The server sends
  `SyncStatus` only after an `Update`, not after the initial sync, so the provider's
  counter (set to 1 by `startSync`) never returns to 0. Invisible today (the FE does
  not surface `hasUnsyncedChanges`; the connection indicator uses `WebSocketStatus`/
  `synced`). Deferred to server-side follow-ups for spec correctness.

## Rollback (back to TipTap Cloud)
Set `TIPTAP_URL` back to the Cloud URL in `refactor-platform-rs/.env`, set the
frontend back (or just stop pointing at the local server), restart both. The
`tiptapAppId` / Cloud secrets were left intact specifically so this is a one-line
revert. Restore the DB snapshot if you took one.
```
