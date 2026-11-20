# Backend behind a PgBouncer connection pool

## Goal

The managed Postgres cluster has a hard per-cluster connection limit. Today the backend
connects to Postgres directly, so every connection in its sqlx pools counts against that
limit, and so does every other client (docs-collab-server, the migrator, admin sessions,
monitoring). Putting the backend behind a PgBouncer pool in transaction mode lets many
client connections share a small, fixed number of server connections, so adding backend
capacity (more concurrency, or more than one backend instance) no longer runs into the
cluster limit.

docs-collab-server already runs this way: its `DOCS_COLLAB_DATABASE_URL` points at a
transaction-mode pool on the same cluster.

## Audit: what transaction mode breaks

In transaction mode PgBouncer hands a server connection to a client for one transaction
only. Anything a client sets on its session can be lost, or can leak to another client.

### Blocker 1: `search_path` is set per connection

`service/src/lib.rs` calls `.set_schema_search_path("refactor_platform")`. SeaORM 2
implements this as an sqlx `after_connect` hook that runs `SET search_path = ...` once on
each new client connection (sea-orm `src/driver/sqlx_postgres.rs`). Behind PgBouncer that
`SET` runs on whichever server connection happened to serve it; later transactions from
the same client can run on server connections that never saw it.

Most entity tables declare `schema_name = "refactor_platform"` and are qualified in
generated SQL. What may still depend on `search_path`:

- Postgres enum casts. Entities declare unqualified enum names (for example
  `enum_name = "role"` in `entity/src/roles.rs`).
- Raw SQL in `entity_api` and `domain` (`Statement::from_sql_and_values`,
  `execute_unprepared`, `find_by_statement`).

All raw SQL found is schema-qualified. Enum casts are not: sea-query renders
`CAST($1 AS "role")` and the `DeriveActiveEnum` macro has no schema option, so enums are
the real dependency.

Fix: make `search_path` a role default on the server, which every new server connection
picks up at login, then drop the per-connection `SET`.

- New migration, scoped to the connecting role and the current database:
  `ALTER ROLE <current_user> IN DATABASE <current_database()> SET search_path = refactor_platform`,
  built in a `DO` block with `format('%I', ...)`. `down` runs the matching `RESET`.
  Taking the role from `current_user` covers environments whose database user is not
  `refactor` (PR previews take it from a secret, `scripts/rebuild_db.sh` accepts any
  user). `IN DATABASE` keeps the default off other databases on the cluster, such as
  docs-collab-server's. A role may set its own per-database defaults, so no admin user is
  needed. This assumes the migrator connects as the same user as the backend, which is
  true today (both use `POSTGRES_USER`).
- Remove `.set_schema_search_path(...)` from `init_database`.
- The pool must use "Use inbound user" (no user override) in the DigitalOcean pool
  settings, so PgBouncer logs in as `refactor` and the role default applies. If an override
  user is ever chosen, that user needs the same default.
- docs-collab-server uses the same role but a different database and fully qualified
  names, so the role default does not affect it.

### Blocker 2: named prepared statements

sqlx prepares every persistent query (the default; SeaORM exposes no way to turn it off)
under a per-connection name (`sqlx_s_N`). `statement-cache-capacity=0` does not change
that: the statement is still named, and with the cache off sqlx never sends `Close`, so
the names pile up on whatever server connection ran them (sqlx-postgres
`src/connection/executor.rs`, `prepare` and `get_or_prepare`). Two client connections
sharing one server connection through transaction-mode PgBouncer then both `Parse`
`sqlx_s_1`, and the second fails with `42P05 prepared statement "sqlx_s_1" already exists`,
or a client's `Bind` lands on a server connection that never prepared the name.

This is safe only if the pool's PgBouncer tracks protocol-level prepared statements
(PgBouncer 1.21 or later with `max_prepared_statements` above zero). DigitalOcean's docs
do not say whether its pools do.

Measured locally (sqlx 0.9 through PgBouncer 1.26.1 in transaction mode, 24 client
connections sharing 4 server connections, 30 distinct statements, against a direct
Postgres control that passed in both modes):

| PgBouncer `max_prepared_statements` | sqlx statement cache on (default) | `statement-cache-capacity=0` |
|---|---|---|
| 0 (tracking off) | errors (`already exists`, `does not exist`) and wrong results | wrong results |
| 200 (tracking on) | 0 errors | wrong results |

That table was measured with sqlx 0.9.1. "Wrong results" means a query returned another
query's rows with no error. From 0.9.1, sqlx with the cache off uses the unnamed
statement, and it sends `Parse` and `Bind` in separate round trips, each ending in `Sync`.
Between them PgBouncer may move the client to another server connection, and it does not
track unnamed statements, so the `Bind` runs whatever unnamed statement that server
connection holds. With sqlx 0.9.0 (the version this workspace locks today) the cache-off
column passes when tracking is on, because 0.9.0 still names every statement; it just
never closes them. Two rules follow:

- Never set `statement-cache-capacity=0` for a connection that goes through PgBouncer.
  It becomes silently unsafe the moment sqlx is upgraded to 0.9.1 or later.
- With the default cache, transaction mode works only if the pool tracks prepared
  statements.

Result: DigitalOcean's pools track prepared statements. The same test through the new
pool showed the same pattern as the local "tracking on" row: no errors with the default
cache, and wrong results with `statement-cache-capacity=0`. The gate below has passed.

Gate: before any code changes, run the same concurrent test against the new pool with
the default cache. If it passes, nothing changes here. If it fails, transaction mode is
not viable with SeaORM/sqlx as they stand; the options are session mode (which does not
lift the connection limit) or a self-hosted PgBouncer with `max_prepared_statements` set.

Do not put `options=` in the pooled URL: PgBouncer rejects unknown startup parameters.
The existing `POSTGRES_OPTIONS` is TLS-only, which is fine.

### Deploy change: the migrator stays direct

`migrationctl up` applies every pending migration in one long transaction of DDL. It keeps
the direct (non-pooled) URL. Only the `rust-app` service moves to the pooled URL.

### Already safe

- Every advisory lock is transaction-scoped (`pg_advisory_xact_lock`); none are
  session-level.
- No `LISTEN` / `NOTIFY`.
- No email, LLM, or HTTP call runs inside an open transaction in `domain` or `entity_api`.
- The "up to three connections per request" (auth session load, protect middleware,
  handler) are acquired one after another, not held together.

### Not changed: the health check on every acquire

SeaORM pings a pooled connection on every acquire (`test_before_acquire`). Behind
PgBouncer each ping costs a server checkout, so pinging only idle connections would be
cheaper, but it would also let connections killed by a database restart fail one request
each. Keep the default until the pool's metrics show the pings matter, then change it in
its own pull request.

### Startup check

The role default exists only for the role that ran the migration. If migrations run as
another role, or the pool logs in as one, enum casts fail on every write. `init_database`
checks that `refactor_platform` is on the connection's `search_path` and refuses to start
otherwise, so a misconfigured deploy fails its health check instead of serving errors.

## Changes

0. Gate: confirm the pool handles named prepared statements (see Blocker 2).
1. Migration setting the connecting role's per-database `search_path` default; remove
   `set_schema_search_path` and add the startup check in `service/src/lib.rs`.
2. `docker-compose.yaml`: `rust-app` gets
   `DATABASE_URL: ${DATABASE_POOL_URL:-<current direct URL>}`. The migrator keeps the
   direct URL. PR previews have no pooler and keep the direct URL through the fallback.
3. `.github/workflows/deploy_to_do.yml`: add `DATABASE_POOL_URL` to the production `.env`
   heredoc from a new `production` environment secret, and note next to
   `DB_MAX_CONNECTIONS` that it now counts client connections to PgBouncer. Note that the
   pooled URL must log in as `POSTGRES_USER`.
4. Verify locally with the `SET` removed after the migration: enum writes (roles, topic
   status) and the raw-SQL paths. Then a PR preview, then watch the first production deploy.

## Sizing rules

- Pool sizes and direct connections all come out of the cluster's connection limit. Leave
  headroom for the migrator, admin sessions, and monitoring; size the backend pool from
  what remains after the collab server's pool.
- Keep `DB_MAX_CONNECTIONS` at roughly twice the backend pool size, not far above it.
  Behind PgBouncer, sqlx's `acquire_timeout` only fires when all *client* connections are
  busy. Queries beyond the pool size wait in PgBouncer's queue, for as long as the queries
  ahead of them take. Keeping the client pool near twice the pool size bounds that queue,
  and requests past it still get a 503.
- The session store's reserved connections (`SESSION_POOL_MAX_CONNECTIONS`) are carved out
  of `DB_MAX_CONNECTIONS` as before and also go through the pool.

## Rollout order

1. Create the pool in DigitalOcean: transaction mode, the backend's database, inbound user.
   Run the prepared-statement test against it (Blocker 2) and stop if it fails.
2. Merge and deploy the code. The migrator sets the role default before the backend
   starts; with no `DATABASE_POOL_URL` set, the backend still connects directly.
3. A role default applies only to server connections opened after the migration.
   PgBouncer closes a server connection after `server_lifetime` (an hour by default),
   sooner when it sits idle, so wait an hour after the migration before switching.
   Add the `DATABASE_POOL_URL` secret (pooled port, pool name as the database name, the
   existing TLS parameters) and adjust
   `DB_MAX_CONNECTIONS`. Redeploy.
4. Rollback: delete the secret and redeploy; the backend falls back to the direct URL.
