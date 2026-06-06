# docs-collab-server: Production Deployment (mini-plan)

Deploy the validated `docs-collab-server` (see `docs-collab-server.md`) to the
DigitalOcean droplet stack so production uses it instead of TipTap Cloud. The crate
and its protocol/CRDT correctness are done; this plan is the deployment, routing,
secrets, and DB-ownership work, plus a short pre-prod hardening pass.

This plan is grounded in the existing infra (Dockerfile, `docker-compose.yaml`,
`nginx/conf.d/*`, `deploy_to_do.yml`, `ci-deploy-pr-preview.yml`, `migration/`). It
suits the overseer + handoff workflow: each phase below ends in a buildable,
independently verifiable change.

**Branch:** the validated crate lives on `feat/docs-collab-server` (local + `origin`),
not `main`. `main`'s `docs-collab-server/` dir is only a leftover
`tests/fixtures/capture` Node helper.

**PREREQUISITE - reconcile branches first.** `feat/docs-collab-server` diverged from
`main` (~33 commits): feat has the collab crate; `main` has the merged TipTap *metrics*
feature (#288/#316). Verified (2026-06-04): all infra/deploy files (nginx, both compose,
both deploy workflows, `Dockerfile`, `entrypoint.sh`, `migration/`, `gateway/tiptap.rs`,
`coaching_sessions.rs`, `src/bin/seed_db.rs`) are byte-identical across the two branches,
and migrations are identical (latest `m20260515_000001`). The ONLY material gap is the
metrics feature, which the Phase 7 importer reuses (`tiptap_metrics.rs` ×4, incl.
`list_all_documents()` and `all_collab_document_names()`) - absent on feat. So: create
the implementation branch **off `feat/docs-collab-server`, then merge `main` into it**
(additive new files -> low conflict risk) BEFORE Phase 7. Doing it up front (before
Phase 1) is cleanest. NOTE: `docs-collab-server` is in `members` but EXCLUDED from
`default-members` (dev/experimental), so all `cargo` verification must pass
`-p docs-collab-server` explicitly or the crate is silently skipped.

## Key simplifications (why this is smaller than it looks)

1. **Reuse the app's existing secrets.** The collab server needs its
   `--jwt-signing-key` to equal the app's `TIPTAP_JWT_SIGNING_KEY` and its
   `--management-auth-key` to equal `TIPTAP_AUTH_KEY`. Those already flow through
   every layer (GitHub secrets, both compose files, both deploy heredocs). So we do
   NOT introduce new app secrets for those keys; we pass the existing ones into one
   more service. The only reused secret whose *value* changes is `TIPTAP_URL` (Cloud
   URL -> internal `http://docs-collab:1234`).

   EXCEPTION (revised 2026-06-05): the database is no longer the shared `DATABASE_URL`.
   The collab server uses its OWN managed Postgres via a dedicated
   `DOCS_COLLAB_DATABASE_URL` plus that cluster's CA cert (`DOCS_COLLAB_SSL_ROOT_CERT`),
   two new production secrets. See the Database placement decision below.
2. **Dedicated collab image.** The collab server ships as its own multi-stage
   `docs-collab-server/Dockerfile` with its own CI build+push job and image tag, kept
   separate from the app image. Config flows entirely through the compose
   `environment:` block (clap `#[arg(long, env)]`), so no `ROLE=collab` branch is added
   to the shared `entrypoint.sh`. (This is a deliberate divergence from a single shared
   image: it trades one extra build for independent versioning and a smaller runtime.)
3. **Path-based routing reuses the existing TLS cert.** Route `wss://myrefactor.com/collab`
   through the current nginx + Let's Encrypt cert (like `/api/sse`), so no new
   subdomain/cert/DNS.
4. **The single-instance constraint already exists.** SSE is already documented as
   single-replica (in-memory). docs-collab-server has the same property; the ops model
   is understood, not new.

## Decisions (settle in Phase 0)

- **Routing:** path-based `/collab` on the existing host (recommended) vs a
  `collab.` subdomain (new cert/DNS). Default: path-based.
- **Database placement (revised 2026-06-05): dedicated managed instance.** The collab
  server gets its OWN managed Postgres cluster (small tier), NOT the shared app DB. It
  connects via a dedicated `DOCS_COLLAB_DATABASE_URL` plus that cluster's CA cert
  (`DOCS_COLLAB_SSL_ROOT_CERT`). Rationale: decide the final home before the import (so
  the ~250 docs land once, in place, with no future live migration), keep all ~22
  connections of the 1 GB app tier for the app, and isolate the high-churn full-blob
  UPSERT workload. A separate MANAGED instance avoids the durability downgrade a
  self-managed container would carry. Frugal alternative if cost-bound: a separate
  database + role on the existing managed cluster ($0, shares the cluster failure
  domain, does not relieve the connection ceiling).
- **DB table ownership (revised): self-bootstrap on the dedicated DB.** On a single-role
  dedicated DB the server's `CREATE TABLE IF NOT EXISTS` bootstrap (`storage.rs`) creates
  `collab_documents` owned by the collab role automatically, so the SeaORM migration plus
  `ALTER ... OWNER TO refactor` dance is unnecessary. The Phase 2 migration is superseded
  (see Phase 2). `DATABASE_SCHEMA=refactor_platform` (the server creates the schema).
- **Compute placement: co-located now, dedicated VPS trigger-gated.** The collab server
  stays on the existing droplet for now (light CPU/RAM at 10s-hundreds of users;
  same-host nginx routing). Because the DB is separate, the server is data-stateless
  (durable state in the managed DB; in-memory docs flush on graceful shutdown), so moving
  it to its own VPS later is cheap (no data move). Triggers: droplet RAM tight under real
  load, or wanting to decouple the app and editing failure domains.
- **Existing-doc import: REQUIRED.** Production must import the existing TipTap Cloud
  documents (~250) into `collab_documents` before cutover so existing coaching notes
  survive the switch. See the import design + Phases 7-8 below. This is on the critical
  path, not optional.
- **Replicas:** single instance (in-memory registry), mirroring the SSE constraint.

## Build execution status (overseer + handoff run, 2026-06-05)

Work branch: `feat/docs-collab-deploy` (= `feat/docs-collab-server` + merged `main`).
Each phase below = one reviewed implementer commit, independently verified by the overseer.

DONE + verified:
- Phase 1 (2887607): `/health` route + env-configurable DB pool. clippy/fmt/4 lib tests green.
- Phase 1b (90c894c): CRITICAL rustls TLS for sqlx (prod needs sslmode=verify-full). rustls linked.
- Phase 2 (561ab6f): `collab_documents` migration owned by refactor. DDL run on scratch DB; matches the crate bootstrap exactly. SUPERSEDED (2026-06-05) by the dedicated-DB self-bootstrap; see Decisions + Phase 2.
- Phase 3 (3b0b399): dedicated `docs-collab-server/Dockerfile`. Release build green; image NOT built (no docker daemon locally).
- Phase 4 (9569367 + f2f3f0c): `docs-collab` compose service (prod TLS + cert mount; preview non-SSL) + nginx `/collab` and `/pr-<NUM>/collab` (both preview confs). compose config validates; nginx -t deferred (no daemon).
- Phase 7 (54ecf8e): TipTap Cloud -> collab_documents importer (export `?format=yjs`, intersect+skip filters, upsert, dry-run). mockito + classify tests pass.

Phase 5 - PARTIALLY done:
- DONE 5a (6b08ab6): `deploy_to_do.yml` optional `deploy_docs_collab` flag (gates the collab
  image pull so app deploys don't restart the stateful collab server) + `DOCS_COLLAB_IMAGE_NAME`
  / `NEXT_PUBLIC_DOCS_COLLAB_URL` env passthrough + nextjs-app compose env. compose validates.
- DONE 5b (a71ac8c): `build-test-push.yml` builds+pushes the collab image to its OWN GHCR
  package `.../<branch>/docs-collab:{latest,<sha>}`, distinct GHA cache scope. YAML parses.
- DONE 5c-compose: `docker-compose.pr-preview.yaml` frontend `NEXT_PUBLIC_DOCS_COLLAB_URL`.
- OUTSTANDING 5c-workflow (`ci-deploy-pr-preview.yml`) + GitHub vars - do this WITH a live
  preview dispatch (Phase 6), not blind. Precise spec:
  * In the `resolve` step (job `build-arm64-image`, ~L619/686): compute
    `DOCS_COLLAB_IMAGE="${BACKEND_IMAGE_REPO}/docs-collab:pr-${PR}"` (mirror the pr/main/override
    branches that set `BACKEND_IMAGE`) and `echo "docs_collab_image=..." >> $GITHUB_OUTPUT`
    (+ a `docs_collab_tags`).
  * Add a build-push step after `Build and push backend image` (~L816): `context: ./backend-src`,
    `file: ./backend-src/docs-collab-server/Dockerfile`, `platforms: linux/arm64`,
    `tags: ${{ steps.resolve.outputs.docs_collab_tags }}`, cache `scope=docs-collab-arm64`,
    `if:` tied to the backend build (same checkout).
  * In the deploy job's preview-env heredoc (~L1142): add
    `DOCS_COLLAB_IMAGE=${{ needs.build-arm64-image.outputs.docs_collab_image }}` and
    `NEXT_PUBLIC_DOCS_COLLAB_URL=<per-PR ws path, e.g. ws://${host}/pr-${PR}/collab>`.
  * GitHub config (UI; only you can do): prod env `vars.DOCS_COLLAB_IMAGE_NAME` (= the pushed
    collab image:tag) + `vars.NEXT_PUBLIC_DOCS_COLLAB_URL` = `wss://myrefactor.com/collab`;
    preview env equivalents. NOTE: keep `DOCS_COLLAB_IMAGE_NAME` STABLE between app deploys; bump
    it + set `deploy_docs_collab=true` together to roll a new collab build.
  Reason deferred: `ci-deploy-pr-preview.yml` is ~1100 lines of conditional orchestration, zero
  local verifiability (no docker/GHA); best modified against real CI feedback in Phase 6.
- Phase 6 (PR-preview rehearsal): operational; needs a real preview deploy + e2e.
- Phase 8 (prod cutover): out of scope by request.

## Critical findings (discovered during build, 2026-06-05)

- **CRITICAL (prod DB TLS):** the collab crate's `sqlx` had NO TLS feature
  (`["postgres","time","runtime-tokio"]`), but every managed DO Postgres needs
  `sslmode=verify-full&sslrootcert=/app/root.crt`. Revised: this applies to collab's OWN
  dedicated cluster too. Its `DOCS_COLLAB_DATABASE_URL` value must carry those params and
  the container must mount that cluster's CA at `/app/root.crt` (via
  `DOCS_COLLAB_SSL_ROOT_CERT`). Fix: `tls-rustls` in the crate's `sqlx` features
  (Phase 1b, done) + the cert mount and SSL params (Phase 4/5). Does NOT affect PR preview
  (local non-SSL Postgres), so Phase 6 rehearsal would not catch it. Blocks Phase 8 (prod
  cutover).
- **TIPTAP_URL flip is Phase 8, not Phase 4.** Standing up the collab service (Phases
  4-6) must NOT repoint `rust-app`'s `TIPTAP_URL` at `http://docs-collab:1234` - that is
  the cutover, done in Phase 8 after the import. Phase 4 adds the service + routing only.
- **Two preview nginx configs.** `nginx-preview/pr-previews.conf` is the deployed
  preview config (mounted by `docker-compose.nginx-preview.yaml`); `nginx/conf.d/
  pr-previews.conf` is the canonical source. Both need the `/pr-N/collab` route.
- **Healthcheck tooling.** The minimal collab runtime image has no curl/wget/bash; a
  compose `/health` healthcheck needs an HTTP client added to the Dockerfile runtime.

## Phases

### Phase 0 - Decisions + readiness checklist (no code)
Record the four decisions above in this doc. Readiness status:
- **DB placement and ownership (revised): RESOLVED.** The collab server now uses its OWN
  managed Postgres (`DOCS_COLLAB_DATABASE_URL`), not the shared `DATABASE_URL`. On a
  single-role dedicated DB, the server's `CREATE TABLE IF NOT EXISTS` bootstrap creates
  `collab_documents` owned by the collab role automatically, so the cross-user
  `OWNER TO refactor` concern does not arise (it existed only because the shared DB mixes
  `doadmin` and `refactor`). `collab_documents` is a plain table (no custom PG type), so
  the type-ownership gotcha also does not apply.
- **New-cluster CA cert (readiness).** Provision the dedicated managed cluster, download
  its CA cert, and set the `DOCS_COLLAB_DATABASE_URL` (value carrying
  `?sslmode=verify-full&sslrootcert=/app/root.crt`) and `DOCS_COLLAB_SSL_ROOT_CERT` (host
  path to that CA cert) production secrets before the cutover deploy.
- **Cert covers WS route — CONFIRMED.** Apex `myrefactor.com` server block exists
  (`nginx/conf.d/refactor-platform.conf:77-83`) with a valid cert; it's the same block
  `/api/sse` lives in. `location /collab` goes there, reusing the cert. No new
  cert/DNS.
- **Service name + port — CONFIRMED.** No existing use of `docs-collab` or `1234`. All
  backend services share `backend_network`, so nginx resolves `http://docs-collab:1234`
  via Docker DNS; no host port needs publishing in prod.

### Phase 1 - Code prep in docs-collab-server
- Add a **`GET /health`** route to `rest.rs`/`build_router` returning 200 (the load
  balancer / compose healthcheck needs a real backend check, not just nginx's static
  200).
- **Tune the Postgres pool**: add `DB_MAX_CONNECTIONS`/`DB_MIN_CONNECTIONS` clap fields
  (the app already uses these names) and apply via `PgPoolOptions` instead of the sqlx
  defaults (currently a bare `PgPoolOptions::new().connect()`, max 10). The collab pool
  is a distinct `sqlx` pool from the app's sea-orm pool; only the var *names* are shared.
  Revised (2026-06-05): the default is now `DB_MAX_CONNECTIONS=4` / `DB_MIN_CONNECTIONS=1`
  (the collab workload is a load SELECT plus debounced UPSERTs, so 4 is ample), set via the
  clap default and the compose fallback in both compose files.
- Confirm the server reads all config from **process env** (it does, via clap) so the
  compose `environment:` block is sufficient; no `.env` load needed. Log level is
  `RUST_LOG` (read by `tracing_subscriber::EnvFilter` in `main.rs`, default `info`);
  wire it explicitly (prod `info`, preview `docs_collab_server=debug,info`).
- Verify: `cargo test -p docs-collab-server` still green; `clippy`/`fmt` clean.

### Phase 2 - DB migration (own the table)
**SUPERSEDED (revised 2026-06-05).** With collab on its OWN dedicated DB, the server's
`CREATE TABLE IF NOT EXISTS` bootstrap creates and owns `collab_documents` there, so this
SeaORM migration is not needed in production. The already-merged migration
(`m20260604_000000_create_collab_documents.rs`, commit 561ab6f) would otherwise leave an
orphan unused table in the SHARED app DB. User decision: leave it (harmless empty table) or
remove it and unregister from `migration/src/lib.rs` (cleaner, but only if it was never
applied to prod, since removing an applied migration is discouraged). Original intent kept
below for reference.

- Add `migration/src/mYYYYMMDD_..._create_collab_documents.rs` creating
  `refactor_platform.collab_documents` (`name TEXT PRIMARY KEY`, `state BYTEA NOT
  NULL`, `updated_at TIMESTAMPTZ NOT NULL DEFAULT now()`) with `ALTER TABLE ... OWNER
  TO refactor`, plus a `down` that drops it. Register it in `migration/src/lib.rs`.
- Verify: run the migrator locally against a scratch DB; confirm table + ownership.

### Phase 3 - Dedicated image
- Add `docs-collab-server/Dockerfile`: a multi-stage build (cargo-chef cook, then
  `cargo build --release -p docs-collab-server`) producing a thin runtime stage whose
  `CMD`/`ENTRYPOINT` is the binary directly. No shared `entrypoint.sh` / `ROLE` branch.
- All config arrives as process env in the compose `environment:` block (clap reads it):
  `DATABASE_URL`, `DATABASE_SCHEMA`, `JWT_SIGNING_KEY` (= `${TIPTAP_JWT_SIGNING_KEY}`),
  `MANAGEMENT_AUTH_KEY` (= `${TIPTAP_AUTH_KEY}`), `BIND_ADDR=0.0.0.0:1234`,
  `PERSIST_DEBOUNCE_MS`, `IDLE_EVICT_SECS`, `DB_MAX_CONNECTIONS`, `DB_MIN_CONNECTIONS`,
  `RUST_LOG`.
- CI: add a build+push job for this image (its own tag), mirroring the app image job.
- Verify: image builds; `docker run` with the env set starts and serves `/health`.

### Phase 4 - Compose + nginx (prod and PR preview)
- **`docker-compose.yaml`**: add a `docs-collab` service (its own dedicated image,
  env mapping `JWT_SIGNING_KEY: ${TIPTAP_JWT_SIGNING_KEY}` and
  `MANAGEMENT_AUTH_KEY: ${TIPTAP_AUTH_KEY}` plus the DB vars, `BIND_ADDR`, timeouts,
  `RUST_LOG`), `depends_on: migrator`, `networks: [backend_network]`, single replica,
  a compose `healthcheck` hitting `/health`. **Change `rust-app`'s `TIPTAP_URL` to
  `http://docs-collab:1234`** (internal REST).
- **`docker-compose.pr-preview.yaml`**: same service, `platform: linux/arm64/v8`,
  networks `default` + `preview-ingress`, a unique per-PR port, `depends_on: migrator
  (service_completed_successfully)`.
- **`nginx/conf.d/refactor-platform.conf`**: add a `location /collab` that
  `proxy_pass http://docs-collab:1234/;` with the **WebSocket upgrade headers**
  (`proxy_http_version 1.1`, `Upgrade $http_upgrade`, `Connection "upgrade"`) and
  long-lived settings (`proxy_buffering off`, `proxy_read_timeout 24h`), mirroring the
  existing `/api/sse` + Next.js HMR blocks.
- **`nginx/conf.d/pr-previews.conf`**: add `/pr-(\d+)/collab` with the dynamic
  upstream `pr-${pr_number}-docs-collab-1:1234` and the same WS settings.
- Verify: `docker compose config` validates; locally bring up the stack and confirm a
  WS connects through nginx.

### Phase 5 - Env/secret wiring through the deploy layers + frontend URL
Per the project's CRITICAL passthrough rule, wire every layer (most values already
exist; this is mostly referencing them for the new service):
- `deploy_to_do.yml` and `ci-deploy-pr-preview.yml` heredocs: set the collab service's
  env (referencing existing `TIPTAP_JWT_SIGNING_KEY`/`TIPTAP_AUTH_KEY`, the DB vars,
  `BIND_ADDR`, timeouts). **Change the `TIPTAP_URL` value** in the GitHub `production`
  and `PR_PREVIEW_*` environments to the internal collab URL.
- **NEW dedicated-DB secrets (revised 2026-06-05):** `DOCS_COLLAB_DATABASE_URL` and
  `DOCS_COLLAB_SSL_ROOT_CERT` are added to the `deploy_to_do.yml` production heredoc (done)
  and must be set as `production` environment secrets. PR preview keeps using its local
  Postgres container, so no preview passthrough is required for these; only production
  points at the dedicated managed cluster.
- **Frontend**: add `NEXT_PUBLIC_DOCS_COLLAB_URL` to the frontend compose env and both
  deploy heredocs (prod: `wss://myrefactor.com/collab`; preview: the per-PR path), and
  to the GitHub vars. (Frontend PR refactor-platform-fe#409 already consumes it.)
- Verify: a dry-run render of each heredoc shows the new/changed values.

### Phase 6 - PR-preview validation (full-stack rehearsal)
Deploy a PR preview and run the local-e2e checks against the preview URL (provision,
connect, two-window sync, persistence, delete, authz). This rehearses the image,
compose, nginx WS route, and env wiring end to end before touching production.

### Phase 7 - Build + test the Cloud importer
Most of the Cloud-side API is already verified and partly built (see import design).
Net-new work is small:
- Add an **export method** to the TipTap gateway: `GET /api/documents/{name}?format=yjs`
  → `Vec<u8>` (raw `Y.encodeStateAsUpdate` v1 bytes), reusing the existing read-only
  client + auth in `domain/src/gateway/tiptap_metrics.rs` (which already has bounded
  timeouts). The existing `gateway::tiptap.rs` client has only create/delete.
- Add the importer binary at `src/bin/import_collab_docs.rs` (auto-discovered, like
  `src/bin/seed_db.rs`), with a dry-run mode.
- **The one irreducible live check:** prove the round-trip on ONE real prod doc
  (read-only) before bulk — export `?format=yjs` bytes → store in a scratch
  `collab_documents` row → confirm docs-collab-server loads + reconstructs content via
  `Update::decode_v1` + `apply_update`. Docs say `format=yjs` is exactly this shape;
  verify empirically anyway.

### Phase 8 - Production cutover (import + flip)
In a low-traffic maintenance window: run the bulk import, verify counts + spot-check a
few docs, then flip the app `TIPTAP_URL` to `http://docs-collab:1234` and the frontend
`NEXT_PUBLIC_DOCS_COLLAB_URL` to `wss://myrefactor.com/collab`, redeploy, and verify
real sessions load their prior content. Watch logs/metrics. Keep the `TIPTAP_URL`
revert ready (see Rollback for its time limit).

## Importing existing TipTap Cloud documents

Existing Cloud-hosted Yjs documents must land in `collab_documents` before clients
switch over, or those notes appear empty. The Cloud API shape below is CONFIRMED
(TipTap docs + the already-merged `feat/288-tiptap-document-metrics` gateway); only the
per-doc round-trip needs an empirical check in Phase 7.

**Reuse surface (already built by the metrics feature):**
- `domain/src/gateway/tiptap_metrics.rs::Client::list_all_documents()` — paginates
  `GET /api/documents?skip&take` → `[{name, size, archived}]`. `name` ==
  `coaching_sessions.collab_document_name`. Unit-tested.
- `entity_api::tiptap_metrics::all_collab_document_names(db)` — every non-null
  `collab_document_name` from coaching sessions.
- Auth: raw secret (`TIPTAP_AUTH_KEY`) in the `Authorization` header, no `Bearer`.

**Mechanism:**
1. **Enumerate (intersection).** Iterate the Cloud list (`list_all_documents()`) and
   keep only names that also appear in `all_collab_document_names(db)` — imports exactly
   the docs tied to real coaching sessions, dropping Cloud-side orphan/test docs. Then
   pre-skip any doc with `archived == true` or `size == 0` (cheap filter on the list
   payload, before any export call).
2. **Export each doc.** `GET /api/documents/{name}?format=yjs` returns the raw
   `Y.encodeStateAsUpdate` v1 binary update — byte-identical to what
   `collab_documents.state` holds, so it hydrates directly via docs-collab-server's
   `Update::decode_v1` + `apply_update`. NO format conversion needed. (New gateway
   method; see Phase 7.)
3. **Upsert into `collab_documents`** (name, state). Idempotent by name, so re-running
   is safe. Skip 404s and (via the list pre-filter) empty/archived docs.
4. **Dry-run mode** reporting counts (found / exported / would-write / skipped) with no
   writes; run first against staging or a DB copy.

**Cutover sequence (order matters):**
1. docs-collab-server is deployed but the app still points `TIPTAP_URL` at Cloud (live
   traffic unchanged). The importer reads Cloud directly via the Cloud URL + auth,
   independent of the app's config.
2. In the maintenance window, run the importer (bulk export Cloud -> write
   `collab_documents`). Because it is an idempotent upsert and the docs are small,
   re-running all ~250 at the window is cheap and removes any need for delta tracking.
3. Flip `TIPTAP_URL` and `NEXT_PUBLIC_DOCS_COLLAB_URL`, redeploy. Clients now connect to
   the self-hosted server, which hydrates from the imported state.
4. Spot-check a few real sessions for prior content.

**Lost-edit window:** edits made on Cloud between the import and the flip would be
lost. Mitigation: run the bulk import immediately before the flip (it supersedes any
earlier dry run) and keep the window short. For bounded coaching concurrency this is a
brief, low-risk window; announce it if needed.

## Pre-prod hardening (fold in here or track as follow-ons)
From the plan's "Known gaps": idle keepalive + `SyncStatus`-after-initial-sync,
DELETE-evict of a live doc, constant-time management-auth compare. Plus: cap max WS
frame size, add per-token/IP connection limits, a WS `Origin` check, surface
persist-failure metrics, and run the env-gated integration tests in CI. None block a
*new-docs-only* beta; revisit before broad rollout.

## Rollback
Single-value revert: set the app's `TIPTAP_URL` back to the TipTap Cloud URL and the
frontend `NEXT_PUBLIC_DOCS_COLLAB_URL` back to Cloud (or unset), redeploy. The
`tiptapAppId` and Cloud secrets are left intact specifically so this stays one step.
**Time limit:** this clean revert holds only until users start editing on the
self-hosted server. After that, post-cutover edits live in `collab_documents` (not
Cloud), so a late rollback needs a reverse export (collab_documents -> Cloud) or those
edits are lost. Keep the maintenance window tight to bound this.

## Out of scope
Horizontal scaling (Redis pub/sub for multi-node fan-out), and load-test tuning beyond
setting `ulimit`/`LimitNOFILE` for the service.

**Deferred, trigger-gated (not out of scope, just not now):** moving the collab SERVER to
its own VPS, which is cheap once the DB is separate (no data move; repoint the nginx
upstream + frontend WS URL). Triggers: droplet RAM tight under real load, or decoupling the
app and editing failure domains. Redis fan-out only near low-thousands of concurrent
live-doc editors.
