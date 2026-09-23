# SQLite tier: in-memory database tests

The **SQLite tier** runs our real queries against an in-memory SQLite database inside the
test process. It exists because SeaORM's `MockDatabase` never executes SQL: it hands back
whatever rows the test queued, whatever query the code sent. A mock test therefore cannot
tell whether a query selects the right rows or whether a write left the right values in
the table.

## Choosing: mock or SQLite

Default to the mock (`#[cfg(feature = "mock")]` modules, run with
`cargo test -p entity_api -p domain -p web --features "domain/mock,web/mock"`). It is the
right tool for:

- control flow: which calls happen, in what order, what is skipped after a failure
- statement order inside a transaction, and that a statement asks for a SQL feature (for
  example that a claim sends `FOR UPDATE`)
- error mapping and response shapes

Add a SQLite test when correctness depends on the database's answer:

- **which rows a query selects**: filters, cutoffs, `IN` lists, ordering, limits
- **what a write leaves in the table**: re-read the row after an `UPDATE` or `INSERT`;
  the model a write returns proves nothing, because the mock returns the model you gave it

The deciding check: plant the bug you are worried about. If the mock suite still passes, the
behaviour needs a SQLite test. Keep the assertions a mock can make in the mock suite; the
SQLite test covers only what the mock cannot see.

## What SQLite cannot prove

Production runs Postgres. These gaps stay covered some other way:

- **Row locks and concurrency.** SQLite has no row locks, and SeaQuery silently drops
  `FOR UPDATE` for SQLite. A lock-dependent race is covered by a mock test asserting the
  `FOR UPDATE` is sent inside the right transaction; that Postgres honours it is not ours to
  test.
- **Postgres-only SQL.** Advisory locks (`pg_advisory_xact_lock`), enum types, JSONB
  operators and similar cannot run. Keep those code paths out of SQLite tests.
- **The migration's schema.** The table is built from the entity, not the migration, so
  database defaults (`gen_random_uuid()`, `now()`), constraints, triggers and cascades are
  absent. Seed every column explicitly, primary key included. Foreign keys are off, because
  only the tables under test exist.
- **Column types.** UUIDs and timestamps are stored as text. Timestamp comparisons are
  correct only because every value is written through SeaORM in UTC in one format; seed
  timestamps through SeaORM, never as raw SQL strings.

## Writing a SQLite test

- Put the tests in a sibling file named `<module>_sqlite_tests.rs` and wire it from the
  module under test:

  ```rust
  #[cfg(test)]
  #[path = "<module>_sqlite_tests.rs"]
  mod sqlite_tests;
  ```

  Gate it on `#[cfg(test)]` only: never on the `mock` feature, never `#[ignore]`. The tier
  then runs in plain `cargo test`, locally and in CI, with no database to install.
- Get a database from `crate::sqlite_test_support::database()` (in `domain`). To test
  another table, create it in that harness from its entity the same way.
- Wrap every test body in `within_time_limit(async { ... }).await` from the same module, a
  5 minute ceiling per test. A test returning `Result` ends its block with
  `Ok::<(), Error>(())` so `?` inside it knows the error type.
- Seed through an `ActiveModel` with every column set. Assert on state re-read from the
  table, and on files in the store where objects are involved.
- Run the tier on its own with `cargo test -p domain sqlite_tests`.

## Harness workarounds (SeaORM 1.1)

Each is load-bearing; removing one breaks the tier in a way that looks unrelated.

- **One connection, 1 second acquire timeout.** Every `sqlite::memory:` connection is its own
  database, so the pool is pinned to exactly one connection. Code that queries through the
  pool while holding an open transaction then waits for a second connection that never frees
  up. The timeout turns that wait into a failed test instead of a CI job stuck for hours; keep
  it short, because a loop hits it once per row (the purge's 501-row cap test would take
  over 40 minutes at 5 seconds). Per-row waits still add up, so `within_time_limit` also caps
  each whole test at 5 minutes. A failure that reads as a pool timeout, or as that limit,
  means a query escaped its transaction. That makes the one connection a guard in its own
  right: the mock records a statement into the open transaction whichever handle sent it,
  so only this tier catches a query run on the pool instead of the transaction.
- **`ATTACH DATABASE ':memory:' AS refactor_platform`.** Entities name the
  `refactor_platform` schema, which SQLite resolves as an attached database.
- **`AUTOINCREMENT` stripped** from the generated `CREATE TABLE`. SeaORM 1.1 marks every
  primary key auto-increment, which SQLite rejects on our UUID keys.
- **`sqlite-use-returning-for-3_35`**, a dev-only `sea-orm` feature in `domain/Cargo.toml`.
  Without it SeaORM skips `RETURNING` on SQLite, and inserts plus
  `update_many().exec_with_returning()` (used by `restore` and `soft_delete`) fail.

SQLite is a dev-dependency only; the production binary's `sea-orm` features contain no
SQLite. Check with
`cargo tree -p refactor_platform_rs -e normal,features -i sea-orm | grep sqlite`.

The SeaORM 2.x upgrade (#424) is expected to replace the hand-built table with schema sync
and may retire some of these workarounds.

## Current coverage: the coaching session image purge

The purge permanently destroys images, so it was the first user of this tier. The tests live
in `domain/src/jobs/coaching_session_image_purge_sqlite_tests.rs` and
`domain/src/coaching_session_image_sqlite_tests.rs`.

These tests pin the purge's current rule, that `deleted_at` alone decides what is
destroyed. #423 may add a second condition (skip images the note still uses); its tests
belong beside these.

### Mutation audit (2026-09-23)

Each row planted one bug in the purge path, ran both suites, then restored the code. Counts
are failing tests. **Missed** means the bug would have shipped. Re-run this audit after any
change to the purge, its queries or this harness, and whenever the SeaORM upgrade (#424)
lands: plant each change by hand, confirm the tier goes red, restore.

| # | Planted bug | Mock suite | SQLite tier |
|---|---|---|---|
| M1 | Purge ignores the grace period (`cutoff = now`) | **missed** | caught (2) |
| M2 | Grace sign flipped (`now + grace`) | **missed** | caught (2) |
| M3 | Scan cutoff comparison reversed (`lt` to `gt`) | caught (1) | caught (4) |
| M4 | Claim drops its still-due check | caught (1) | caught (1) |
| M5 | Claim drops `FOR UPDATE` | caught (2) | missed: SQLite has no locks |
| M6 | Claim runs on the pool, outside the transaction | **missed** | caught (4) |
| M7 | Row deleted before its object | caught (2) | not flagged: the rollback keeps the row, so nothing is lost |
| M8 | Session-key filter inverted (`is_in` to `is_not_in`) | **missed** | caught (1) |
| M9 | Restore leaves `deleted_at` set | **missed** | caught (1) |
| M10 | Removal writes a timestamp 30 days in the past | **missed** | caught (1) |
| M11 | Scan cap removed | caught (1) | caught (1) |
| M12 | Scan order reversed (newest first) | caught (1) | caught (1) |

Every planted bug is caught by at least one suite. The two tiers are complementary, not
redundant: M5 is guarded only by the mock, and M1, M2, M6, M8, M9 and M10 only by SQLite.
Deleting either suite reopens real ways to destroy a user's images.
