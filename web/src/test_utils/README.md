# `web::test_utils`

Support code shared by `web`'s tests. It compiles only under `cfg(test)`, so none of it
reaches the production binary.

This directory holds what several tests share, not the tests themselves. A test lives beside
the module it covers: mock tests in `<module>_tests.rs`, SQLite tests in
`<module>_sqlite_tests.rs`, each pulled in from the module with `#[path]`.

| Module | Provides | For |
|---|---|---|
| `sqlite` | `TestApp` (the production route table on in-memory SQLite, seeded with `domain`'s `seed_access_world`, signed in per `Caller`, with local Tiptap, Resend, AssemblyAI and object store stand-ins), `Reply`, `Listener`, `NON_PARTICIPANTS`, the id helpers (`ids`, `ids_in`, `ids_by_key`, `field_ids`) and the assertions more than one test file uses | route tests in `*_sqlite_tests.rs` |

## Which kind of test

Write a **mock test** by default: a handler's control flow, error mapping, or what it passes
to `domain`, with `MockDatabase` and a hand-built router.

Write a **SQLite route test** with `TestApp` when the answer depends on real rows and the
real route table: which caller a route lets through, which rows it returns, what a write
leaves in the table, or who an email or event reaches. It sends requests through
`router::define_routes` with production's sign-in layers, so moving a check between
middleware and an extractor cannot break it. When to choose the SQLite tier and what it
cannot prove: `docs/test-plans/sqlite_integration_testing.md`.

Each route test names the access-matrix row it checks in a one-line doc comment
(`/// ACT-32`); `docs/test-plans/relationship_access_manual_testing.md`, section 6, maps
every row to its test.

## Running them

- SQLite route tests: `cargo test -p web sqlite_tests` (they also run in plain `cargo test`)
- Mock tests: `cargo test -p entity_api -p domain -p web --features "domain/mock,web/mock"`
