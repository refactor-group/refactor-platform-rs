# `domain::test_utils`

Support code shared by `domain`'s tests: harnesses, fakes and fixtures. It compiles only
under `cfg(test)` or the `test-sqlite` feature, which `web` enables as a dev-dependency
feature only, so none of it reaches the production binary.

This directory holds what several tests share, not the tests themselves. A test lives beside
the code it covers: mock tests in that module's `tests` module or `<module>_tests.rs`, SQLite
tests in `<module>_sqlite_tests.rs`. Living inside the crate is what lets them reach private
items, which `domain/tests/` could not.

| Module | Compiled when | Provides | For |
|---|---|---|---|
| `mock` | `domain`'s tests with the `mock` feature | `recording_publisher`, `both_participants_are_members` | mock database tests (the default) |
| `sqlite` | `domain`'s tests, or `test-sqlite` | `database`, `within_time_limit`, `seed_user`, `seed_organization`, `seed_coaching_session`, `seed_note`, `seed_action`, `seed_agreement`, `seed_topic`, `seed_lapsed_membership`, `foreign_key_violation` (with `mock`) | SQLite tier tests |
| `sqlite::access_world` | as `sqlite` | `seed_access_world` (the access matrix's fixture), `mirror_migration_defaults` (in `sqlite`) | `web`'s SQLite route tests (`web/src/test_utils/sqlite.rs`) |

## Which kind of test

Write a **mock test** by default: control flow, call order, statement order inside a
transaction, error mapping.

Write a **SQLite tier test** when correctness depends on which rows a query selects or what
a write leaves in the table. The mock never executes SQL, so it cannot see either. When to
choose it, what it cannot prove and how to write one:
`docs/test-plans/sqlite_integration_testing.md`.

## Running them

- Mock tests: `cargo test -p entity_api -p domain -p web --features "domain/mock,web/mock"`
- SQLite tier alone: `cargo test -p domain sqlite_tests` (it also runs in plain `cargo test`)
