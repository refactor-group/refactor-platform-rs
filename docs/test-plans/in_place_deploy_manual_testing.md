# Test Plan: In-Place Deploy, All-or-Nothing Migrations, Rollback

Verify that a production deploy replaces only the containers that changed, never
runs the old backend against a newer schema, applies a deploy's migrations all or
nothing, and that `rollback_do.yml` restores the previous images only when that is
safe.

> [!IMPORTANT]
> Nothing here has automated coverage. The migration behavior is Postgres-only (the
> SQLite tier has no enum types and SeaORM skips per-migration transactions there),
> and the deploy and rollback scripts only run on the production host.

## 1. Prerequisites

- Part A: local Postgres (`./scripts/rebuild_db.sh` prerequisites), `cargo`, and a
  scratch database you can drop. Never point these steps at your dev database.
- Part B: permission to run `deploy_to_do.yml` and `rollback_do.yml`, and SSH to the
  production host. Pick a quiet time: B2 and B4 replace the backend.

## 2. Part A: all-or-nothing migrations (local)

Set up a scratch database:

```bash
psql -h localhost -U postgres -c "CREATE DATABASE refactor_migration_check OWNER refactor"
psql -h localhost -U refactor -d refactor_migration_check -c "CREATE SCHEMA refactor_platform"
export DATABASE_URL=postgres://refactor:password@localhost:5432/refactor_migration_check
```

**A1. A fresh database applies the full history.**
`cargo run -p migration -- up -s refactor_platform`
**Pass:** every migration applies. (A fresh database runs one transaction per
migration: its history adds enum values and uses them in later migrations.)

**A2. A failed run on an existing database applies nothing.**
1. `cargo run -p migration -- down -n 3 -s refactor_platform`
2. Make the last of the three fail: create the object it creates, for example the
   enum type its `up` adds (`CREATE TYPE refactor_platform.<type> AS ENUM ('x')`).
3. `cargo run -p migration -- up -s refactor_platform`

**Pass:** the command exits 1 on the third migration, `seaql_migrations` still has 3
fewer rows than A1, and anything the first two drop or add is unchanged.

**A3. The retry applies all three.** Drop the object from A2 step 2 and rerun `up`.
**Pass:** all three apply.

**A4. `status` refuses a database ahead of the binary.**
`INSERT INTO refactor_platform.seaql_migrations VALUES ('m29990101_000000_future', 0)`,
then `cargo run -p migration -- status -s refactor_platform`.
**Pass:** exits 1 with "this migration has been applied but its file is missing".
This is the check `rollback_do.yml` and the deploy's automatic rollback rely on.

Clean up: `psql -h localhost -U postgres -c "DROP DATABASE refactor_migration_check"`.

## 3. Part B: production deploy and rollback

**B1. A deploy with no new images replaces nothing.** Run `deploy_to_do.yml` on
`main` when `main:latest` already runs.
**Pass:** the log shows `Changed images: none`, no `Recreate` lines, both health
checks pass, and `docker ps` shows the same container ages as before.

**B2. A backend deploy replaces only the backend.** Run it after a backend-only merge.
**Pass:** `Changed images: rust-app`; the backend is stopped before the migrator
runs; `nextjs-app` and `nginx-reverse-proxy` keep their age; `nginx reloaded`;
health passes; `/home/deploy/rollback/manifest` lists `rust-app` and
`docker images refactor-rollback/rust-app:previous` exists.

**B3. Rollback refuses when the database moved on.** After a deploy that added a
migration, run `rollback_do.yml` with `force` off.
**Pass:** it exits 1 naming the missing migration, and nothing is replaced.

**B4. Rollback restores the previous backend.** After a deploy with no new
migration, run `rollback_do.yml`.
**Pass:** the previous image runs, `BACKEND_IMAGE_NAME`'s tag points at it
(`docker image inspect -f '{{.Id}}'`), health passes, and the manifest is gone. Run
`deploy_to_do.yml` again to return to `main:latest`.
