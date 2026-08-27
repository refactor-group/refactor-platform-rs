# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Also read `.claude/CLAUDE.md` first — it defines mandatory file consultations (coding standards, PR instructions, migration rules) and repo-wide rules that apply before any edit.

## Commands

### Build, run
- `cargo check` — verify compilation (preferred over `cargo build` for quick iteration)
- `cargo run -- <flags>` — run the backend directly; see `docs/setup.md` for required env vars/flags (Tiptap collab, Resend email, meeting-recording credentials)

### Tests
- The mock-gated suite (SeaORM `MockDatabase`-backed tests) needs its own invocation: `cargo test -p entity_api -p domain -p web --features "domain/mock,web/mock"`. **Never** run `cargo test --workspace --features mock`: enabling `sea-orm/mock` workspace-wide drops `DatabaseConnection: Clone`, which breaks the main binary build.
- Single test: `cargo test -p <crate> <test_name>` (add the `--features` flag above if the test lives behind `#[cfg(feature = "mock")]`)
- `cargo test --release` — the slower path CI runs before production image builds

### Database
- `./scripts/rebuild_db.sh [db] [user] [schema]` — create/reset the local Postgres db, user, and schema (defaults: `refactor_platform` / `refactor` / `refactor_platform`, password `password`)
- `cargo run --bin seed_db` — seed local test data
- `DATABASE_URL=postgres://refactor:password@localhost:5432/refactor_platform sea-orm-cli migrate up -s refactor_platform` — run migrations manually against a running Postgres
- Generating an entity from an existing table requires one `--ignore-tables <table>` flag per *other* table (see README for the full `sea-orm-cli generate entity` invocation)

## Architecture

### Layered request flow and error propagation
Logical layering is `web` (Axum handlers/routes) → `domain` (business logic) → `entity_api` (CRUD on entities) → `entity` (SeaORM models). This is enforced at the Cargo level, not just by convention: `web/Cargo.toml` depends on `domain` only and has no dependency on `entity_api` or `entity` at all, so reaching for an `entity_api` type from `web` is a compile error, not just a lint. Errors flow the same chain in reverse — `entity_api::Error` → `domain::Error` → `web::Error` → HTTP response — via `From` impls at each boundary; see `.claude/coding-standards.md` for the conversion rules (in particular: entity error types must reach `domain::Error` exclusively through `From<EntityApiError>`, never a standalone `impl From<entity::*>`).

### Authorization: resource-scoped extractors, not ad-hoc middleware
New route authorization is standardized on Axum `FromRequestParts` extractors in `web/src/extractors/`, named `<resource>_access` (e.g. `coaching_session_access.rs`, `organization_admin_access.rs`, `super_admin_access.rs`, `coaching_relationship_access.rs`). An extractor loads the resource and checks the authenticated user's role/relationship against it, rejecting before the handler body runs — check for an existing extractor before writing a new access check. `web/src/protect/` holds older role-rule based middleware (`AuthorizationRule` in `protect/mod.rs`, applied via `axum::middleware::from_fn_with_state`); it still guards some routes but is not the pattern for new authorization work.

### Real-time updates: `events` + `sse`
`events` is a dependency-free foundation crate defining `DomainEvent` / `EventHandler` / `EventPublisher`; entity payloads travel as `serde_json::Value` specifically so this crate never depends on `entity` or `domain` (avoids a circular dependency, since `domain` depends on `events`). `sse` consumes `events` and manages exactly one SSE connection per authenticated user via a dual-indexed `DashMap` registry (O(1) lookup by connection and by user) — it is single-instance only, with no cross-process fanout. Delivery is ephemeral: domain code publishes an event, `sse` routes it to a connected user if one exists, and an offline user simply sees fresh data on next page load rather than replaying missed events. See the module doc comments in `events/src/lib.rs` and `sse/src/lib.rs` for the full message-flow.

### Meeting recording/transcription abstraction
`meeting-auth` implements OAuth2 and API-key authentication against meeting providers (Zoom, Google, Recall.ai). `meeting-ai` depends on `meeting-auth` and abstracts recording-bot dispatch, transcription, and analysis behind provider-agnostic traits. Both `domain` and `web` depend on `meeting-ai` directly.

### Further reading
`docs/architecture/` has focused write-ups per subsystem (crate dependency graph with a rendered Mermaid diagram, background jobs, email notifications, MCP server, password reset, throttling) — worth checking before making non-trivial changes to those areas. `docs/setup.md` covers full local/production environment setup, including credentials this repo doesn't need for a plain `cargo check`/`cargo test` loop.
