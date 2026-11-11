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
- **The migration's schema.** The tables are built from the entities by schema sync, not
  from the migrations. Foreign keys are on, and their `ON DELETE` actions come from the
  entity relations, so a cascade test proves what the entity declares, not what the
  migration created.
- **Indexes the entities don't declare.** A composite unique index that exists only in a
  migration (for example an `ON CONFLICT` target) is absent. Create it by hand in the test,
  mirroring the migration.
- **Database defaults.** `gen_random_uuid()`, `now()` and triggers are absent. Seed every
  column explicitly, primary key included.
- **Column types.** Enums and JSONB become `enum_text` / `jsonb_text` columns, which SQLite
  stores as unchecked text: an enum value the Postgres type would reject is accepted. UUIDs
  and timestamps are stored as text. Timestamp comparisons are correct only because every
  value is written through SeaORM in UTC in one format; seed timestamps through SeaORM,
  never as raw SQL strings.

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
- Get a database from `crate::test_utils::sqlite::database()` (in `domain`, see
  `domain/src/test_utils/README.md`). It holds every registered entity's table; seed parent
  rows with `seed_user`, `seed_organization` and `seed_coaching_session`.
- Wrap every test body in `within_time_limit(async { ... }).await` from the same module, a
  5 minute ceiling per test. A test returning `Result` ends its block with
  `Ok::<(), Error>(())` so `?` inside it knows the error type.
- Seed through an `ActiveModel` with every column set. Assert on state re-read from the
  table, and on files in the store where objects are involved.
- Run the tier on its own with `cargo test -p domain sqlite_tests`. A second SQLite module
  beside a module's `sqlite_tests` keeps the suffix (for example `cascade_sqlite_tests`) so
  this filter still finds it.

## Harness

`domain/src/test_utils/sqlite.rs` builds the database. Each piece is load-bearing; removing
one breaks the tier in a way that looks unrelated.

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
- **Schema sync from an explicit entity list.** `database()` registers each entity with
  `get_schema_builder()` and syncs them. It does not use the global schema registry:
  `coaches` and `coachees` are narrower views of the `users` table, and which of the three
  the registry picks to define `users` depends on link order. **A new entity must be
  registered in `database()`**, or its table does not exist in the tier.
- **Foreign keys on** (`PRAGMA foreign_keys = ON`). Every insert needs real parent rows,
  which is what the seed helpers are for: `seed_user`, `seed_organization`, and
  `seed_coaching_session`, which creates a user, an organization, a relationship and a
  session and returns `(session_id, user_id)`.
- **UUID primary keys are `auto_increment = false`** in the entities. Schema sync would
  otherwise emit `AUTOINCREMENT`, which SQLite rejects on a non-integer key.
- **`RETURNING`** comes from `sqlite-use-returning-for-3_35`, a default `sea-orm` feature.
  Without it inserts and `update_many().exec_with_returning()` (used by `restore` and
  `soft_delete`) fail on SQLite.

SQLite is a dev-dependency only: the SQLite driver is enabled in `domain/Cargo.toml`'s
dev-dependencies, and the production binary links none. Check with
`cargo tree -p refactor_platform_rs -e normal,features -i sea-orm | grep sqlx-sqlite`,
which must print nothing. (`sqlite-use-returning-for-3_35` does appear there; it is a
default flag feature that pulls in no SQLite code.)

## Current coverage

- **ON CONFLICT DO NOTHING contract**
  (`domain/src/coaching_relationship_sqlite_tests.rs`,
  `domain/src/coaching_session_goal_sqlite_tests.rs`): a conflicting insert writes nothing
  and reports `RecordNotFound`, and a bulk `exec_with_returning` yields only the rows
  actually written, or none when every row conflicts.
- **Coaching session image cascades**
  (`domain/src/coaching_session_image_cascade_sqlite_tests.rs`): deleting a session removes
  only its own images, and deleting a user removes the images they uploaded.
- **Search visibility** (`domain/src/search_sqlite_tests.rs`): over an access matrix
  across two organizations, the relationships search scopes on (`visible_to`) are exactly
  the ones `grants_access_to` allows, for every caller except super admins, who search
  everything.
- **Nearest sessions and the prior/next lookups**
  (`domain/src/coaching_session_sqlite_tests.rs`): the `/nearest` window keeps the sessions
  closest to `around` on each side, counts a session exactly at `around` as after, breaks
  date ties by id, reads only the requested relationship, and returns nothing to a
  non-participant. With several relationships, each keeps its own window, and one the
  caller isn't part of (or that doesn't exist) adds nothing. The prior/next lookups skip a
  session at the given date.
- **"This and following" series reschedules**
  (`domain/src/coaching_session_series_sqlite_tests.rs`): moving a session onto a slot
  rewrites only that row's date, slot, series and duration, keeps its notes attached, and
  leaves its neighbours alone. The content check flags a session with a note, action,
  agreement or live topic, and nothing else. The series' sessions are walked by date with
  ties broken by id.
- **Working hours on user update** (`domain/src/user_sqlite_tests.rs`): updating only one
  working-hours field keeps the stored other one, and an update whose start would fall
  after the stored end writes nothing, not even the other fields sent with it.
- **Pending reschedules**
  (`domain/src/coaching_session_pending_reschedule_sqlite_tests.rs`): the first change
  records the pre-edit schedule as announced, later ones bump the revision, and an undo
  back to it drops an unclaimed or lease-expired row but keeps a claimed one. A row comes
  due after the quiet period, at the cap, or when its session starts within the cap;
  claims take the oldest first up to the limit and skip rows under a live lease.
  Completion settles only at the claimed revision, otherwise keeps the newer change
  pending from what was sent; a stale claim changes nothing, failures count attempts up to
  giving up, a new change restores the full retry budget, and deleting the session drops
  its row.
- **The pending reschedule sweep**
  (`domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs`): a due change is
  emailed (moved from the last announced start), settled, and stamps notice; a failed send
  leaves the row unclaimed with one more attempt until it is given up; a change already
  reverted settles without an email; drain mode drops due rows unsent without stamping
  notice; and a change that is not yet due is left untouched.
- **A user's own lists skip organizations they left**
  (`domain/src/coaching_relationship_sqlite_tests.rs`,
  `domain/src/coaching_session_sqlite_tests.rs`, `domain/src/action_sqlite_tests.rs`,
  `domain/src/goal_sqlite_tests.rs`): a user's own relationships, sessions (list and
  nearest), actions (both scopes) and goals list only relationships in organizations
  where they still hold a role. A super admin, or a member of every organization involved,
  sees them all.
- **Removed members**: moving a session whose coachee left the organization is refused and
  writes nothing, while retitling or deleting it still succeeds
  (`domain/src/coaching_session_update_sqlite_tests.rs`). A "this and following"
  reschedule that skips a removed coachee settles the moved sessions' pending reschedules
  without stamping notice, one that reached the coachee settles and stamps it, and a failed
  send records nothing (`domain/src/coaching_session_series_sqlite_tests.rs`). Accepting a
  suggested action assigned to a removed coachee is refused
  (`domain/src/transcript_suggestion/review_sqlite_tests.rs`). The organization
  relationship list reports `coach_is_member` and `coachee_is_member` per participant,
  including to an org admin when the coach was removed
  (`domain/src/coaching_relationship_membership_sqlite_tests.rs`). Those fields agree with the
  notify filter for a member, a removed user, a user with a role only in another
  organization, a global super admin and an org admin, and a session read with
  `include=relationship` marks a removed coachee
  (`domain/src/coaching_session_sqlite_tests.rs`).
- **Account delete with coaching history** (`domain/src/user_delete_sqlite_tests.rs`): an
  account in any relationship that has a session is refused with `user_has_coaching_history`
  and the count of such relationships, and nothing is deleted. An account whose relationships
  have no sessions is deleted along with them and their goals. A session that appears after
  the check, forced with a temporary trigger, is refused the same way rather than surfacing a
  foreign key fault. The tests recreate `user_role_changes` with SQLite defaults, since its id
  and timestamp defaults exist only in the migrations.
- **The coaching session image purge**, below.

### The coaching session image purge

The purge permanently destroys images, so it was the first user of this tier. The tests live
in `domain/src/jobs/coaching_session_image_purge_sqlite_tests.rs` and
`domain/src/coaching_session_image_sqlite_tests.rs`.

These tests pin the purge's current rule, that `deleted_at` alone decides what is
destroyed. #423 may add a second condition (skip images the note still uses); its tests
belong beside these.

### Mutation audit

Each row plants one bug in the purge path, runs both suites, then restores the code. Counts
are failing tests. **Missed** means the bug would have shipped. Re-run this audit after any
change to the purge, its queries or this harness: plant each change by hand, confirm the
tier goes red, restore.

Expected outcome (SeaORM 2.0.3, foreign keys on):

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
| M9 | Restore leaves `deleted_at` set | **missed** | caught (2) |
| M10 | Removal writes a timestamp 30 days in the past | **missed** | caught (1) |
| M11 | Scan cap removed | caught (1) | caught (1) |
| M12 | Scan order reversed (newest first) | caught (1) | caught (1) |

Every planted bug is caught by at least one suite. The two tiers are
complementary, not redundant: M5 is guarded only by the mock, and M1, M2, M6, M8, M9 and
M10 only by SQLite. Deleting either suite reopens real ways to destroy a user's images.
