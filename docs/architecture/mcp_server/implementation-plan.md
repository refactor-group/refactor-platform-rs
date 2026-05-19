# MCP Server Implementation Plan
Atomic commits organized inside-out: deepest dependencies first, consumers last.
Each numbered step is a separate commit. Commit before starting the next step. See `commit-strategy.md` for the full commit workflow including validation and message conventions.
## Section 1: Database & Entity Layer (PAT)
Foundation — the PAT table and its SeaORM entity. No consumers yet.
1. **Migration: create `personal_access_tokens` table** — new migration with table definition (id, user_id FK, token_hash, status enum, last_used_at, created_at, updated_at) + partial unique index `UNIQUE (user_id) WHERE status = 'active'`. Run migration to verify.
2. **Entity: `personal_access_tokens` SeaORM entity** — new file `entity/src/personal_access_tokens.rs`. Define `Model`, `Relation` (belongs_to users), `ActiveModelBehavior`. Register in `entity/src/lib.rs`. Verify it compiles.
3. **Update DBML** — add `personal_access_tokens` table and relationship to `docs/db/refactor_platform_rs.dbml`.
## Section 2: Entity API Layer (PAT CRUD)
Data operations on PATs. No callers yet.
4. **Entity API: `personal_access_token` module** — new file `entity_api/src/personal_access_token.rs`. Implement `create(db, user_id, token_hash)`, `find_by_token_hash(db, hash)`, `find_active_by_user(db, user_id)`, `deactivate(db, pat_id)`. Register in `entity_api/src/lib.rs`. Unit tests with mock DB. Note: `find_by_token_hash` is a direct indexed lookup, not a scan — SHA-256 is deterministic, so the middleware hashes the incoming raw token and queries by the hash directly.
## Section 3: Domain Layer (PAT Business Logic)
Hashing, validation, token generation. No web layer yet.
5. **Domain: `personal_access_token` module** — add `sha2` and `rand` to `domain/Cargo.toml` (both already in workspace via `meeting-auth`). New file `domain/src/personal_access_token.rs`. Implement `create_token(db, user_id) -> (raw_token, Model)` (generates random token via `rand::rngs::OsRng`, hashes with `sha2::Sha256`, calls entity_api create, returns raw token + model), `validate_token(db, raw_token) -> Result<users::Model>` (hashes input, looks up by hash, checks status is active, loads user + roles), `deactivate_token(db, pat_id)`. Register in `domain/src/lib.rs`. Unit tests.
## Section 4: PAT REST Endpoints (for UI)
The UI needs to create/show/deactivate tokens. Independent of MCP.
6. **Controller: `pat_controller`** — new file `web/src/controller/pat_controller.rs`. Three endpoints: `POST /users/:id/tokens` (create, returns raw token once), `GET /users/:id/tokens` (show active token metadata, no raw value), `PUT /users/:id/tokens/:token_id/deactivate`. Register in controller mod.rs.
6b. **Protect: `users/tokens`** — new file `web/src/protect/users/tokens.rs`. Authorization middleware for PAT routes: authenticated user can only manage their own tokens (`:id` must match the session user). Uses the existing `Check` trait and `Predicate` pattern from `protect/mod.rs`. Register in `protect/users/mod.rs`.
7. **Router: PAT routes** — add `pat_routes(app_state)` function in `web/src/router.rs`, merge into `define_routes()`. Protected by `require_auth` + `protect::users::tokens` ownership check via `route_layer`.
## Section 5: Add `rmcp` Dependency
Prove the SDK works with the codebase before building on it.
8. **Add `rmcp` to `web/Cargo.toml`** — add `rmcp = { version = "...", features = ["server", "macros", "transport-streamable-http-server"] }`. Verify the workspace compiles with the new dependency.
## Section 6: MCP Module Skeleton
Empty MCP module structure. No tools, no auth, no routes yet.
9. **Create `web/src/mcp/mod.rs`** — declare submodules: `pub(crate) mod auth;`, `pub(crate) mod tools;`. Create empty files for `web/src/mcp/auth.rs` and `web/src/mcp/tools/mod.rs`. Register `pub(crate) mod mcp;` in `web/src/lib.rs`. Verify compiles.
## Section 7: MCP Server Handler (rmcp)
Minimal MCP server that responds to `initialize` and `tools/list` with zero tools. Proves rmcp integration works.
10. **Implement `ServerHandler` for a minimal MCP handler** — in `web/src/mcp/tools/mod.rs`, define `McpToolHandler` struct holding `app_state: AppState` (no user field). Implement rmcp's `ServerHandler` trait via `#[tool_router]` + `#[tool_handler]`. Return server info (name, version, capabilities: tools enabled). No tools registered yet. `tools/list` returns empty. Add a test that constructs the handler and verifies `get_info()` returns expected values.

**User context:** The authenticated user is NOT stored on the handler struct. `rmcp` propagates HTTP request `Parts` into `RequestContext.extensions`, so tool handlers extract the user via `context.extensions.get::<axum::http::request::Parts>()` → `parts.extensions.get::<users::Model>()`. This pattern is used by [`apollo-mcp-server`](https://github.com/apollographql/apollo-mcp-server/blob/ff28390b/crates/apollo-mcp-server/src/server/states/running.rs). No `Arc<RwLock<...>>` bridging needed. Validate that `Parts` propagation works during this step.
## Section 8: MCP Auth Middleware
PAT bearer extraction and validation. Independent of tools.
11. **Implement PAT auth middleware** — in `web/src/mcp/auth.rs`:
    - `validate_pat_from_header(db, headers) -> Result<users::Model>`: extracts `Authorization: Bearer <token>`, calls `domain::personal_access_token::validate_token`, returns the authenticated user with roles.
    - `require_pat_auth`: an Axum middleware function (used via `middleware::from_fn_with_state`). Calls `validate_pat_from_header` and either inserts `users::Model` into request extensions and continues, or short-circuits with 401. Same pattern as existing `require_auth` middleware. The user inserted here is later accessible in tool handlers via `context.extensions.get::<axum::http::request::Parts>()` → `parts.extensions.get::<users::Model>()`.
    - Test with mock DB: valid token proceeds, missing/invalid token returns 401.
## Section 9: MCP Route + Integration
Wire everything together: auth middleware wraps the router containing the nested MCP service.
12. **Create `StreamableHttpService`, wrap with auth, nest into router** — in `web/src/router.rs`, add `mcp_routes(app_state)` function that:
    1. Creates a `StreamableHttpService::new(factory, session_manager, config)` where `factory` is a closure that builds a `McpToolHandler` with `app_state` only (no user — user comes from request context).
    2. Nests the service into a Router via `nest_service("/mcp", mcp_service)`.
    3. Applies the PAT auth middleware as a `.layer(middleware::from_fn_with_state(app_state, require_pat_auth))` on the Router — NOT `route_layer` (which only applies to `.route()` registrations).
    - This matches rmcp's official `simple_auth_streamhttp` example.
    - Integration test: POST to `/mcp` without auth returns 401. POST with valid PAT and `initialize` method returns server info.
## Section 10: Tool — `list_coachees`
First real tool. Coach-only, read-only.
13. **Implement `list_coachees` tool** — add to `McpToolHandler` using `#[tool]` macro. Extracts the authenticated user from `context.extensions.get::<axum::http::request::Parts>()` and accesses `self.app_state` for DB. Calls `domain::coaching_relationship::find_by_user` to get relationships where user is coach. Returns coachee profiles (id, name, email). Test: mock a coach user with two coachees, verify tool returns both.
## Section 11: Tool — `get_coachee`
Rich coachee profile with optional `include`.
14. **Implement `get_coachee` tool** — accepts `coachee_id` (required for coaches, defaults to self for coachees). Authz via `coaching_relationship::is_coach_of`. Returns profile + stat counts. Test: coach can get own coachee, coach cannot get someone else's coachee.
15. **Add `include` support to `get_coachee`** — optional `include` array param (`goals`, `actions`, `notes`). When present, fetches related records filtered to active statuses and appends to response. Test: with `include: ["goals"]`, response contains active goals array.
## Section 12: Tool — `list_sessions`
16. **Implement `list_sessions` tool** — accepts `coachee_id` (required for coaches), optional `date_from`/`date_to`. Authz via coaching relationship membership. Returns session summaries (id, date, meeting_url). Test: returns sessions within date range for authorized user.
## Section 13: Tool — `list_actions`
17. **Implement `list_actions` tool** — accepts optional `coachee_id` (required for coaches, defaults to self for coachees), `coaching_session_id`, `keyword` (searches body), `date_from`/`date_to`, `status`. Authz via `coaching_relationship::is_coach_of`. Returns `ActionResponse` arrays (flattened `actions::Model` + computed `session_url`). Test: keyword filter returns matching actions, session_id filter works, coach without coachee_id returns error, coachee defaults to self.
## Section 14: Tool — `get_session`
18. **Implement `get_session` tool** — accepts optional `session_id`, defaults to latest for the relationship. Authz via session ownership (`find_by_id_with_coaching_relationship` + `includes_user`) or `is_coach_of` when `coachee_id` provided. Returns structured JSON bundle: `SessionResponse` + raw entity arrays for notes, actions, agreements, and linked goals. No server-side LLM — the client LLM summarizes from this data. Test: returns complete session bundle with all related records for an authorized user.
## Section 15: Cleanup & Documentation
19. **Update ADR** — mark Decision section as final, remove any remaining placeholder text.
20. **Update README** — add MCP server section documenting the `/mcp` endpoint, PAT setup, and available tools.
