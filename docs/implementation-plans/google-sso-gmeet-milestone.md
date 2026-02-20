# Google SSO + GMeet Milestone

**Status:** In Progress

**Branch:** `260-feature-milestone-1---google-sso-gmeet`

## Goal

A user can:
1. Connect their Google account to their platform account
2. Add a new or existing Google Meet to a coaching session
3. Launch Google Meet from a coaching session (both coaches and coachees)

## Migration Changes

### What changed

Replaced the prototype `user_integrations` table (hardcoded `google_*`, `recall_ai_*`, `assembly_ai_*` columns) with two focused changes:

1. **`provider` enum + `oauth_connections` table** — Generic, provider-agnostic OAuth credential storage. One row per user-provider pair. Tokens encrypted at the app layer. Row existence = connected; deletion = disconnected.

2. **`meeting_url` + `provider` on `coaching_sessions`** — Moved from `coaching_relationships` to `coaching_sessions`. Each session gets its own meeting link, which works across all providers (Zoom creates unique meetings, Google Meet can reuse or create new).

### What we removed

- `user_integrations` table and migration — prototype with hardcoded provider columns
- `ai_privacy_level` columns on `coaching_relationships` — deferred to AI/recording milestone
- `meeting_recording_tables` migration — deferred to AI/recording milestone
- `coachee_ai_privacy_level` migration — deferred to AI/recording milestone
- `lemur_fields` migration — deferred to AI/recording milestone

### Key design decisions

- **PostgreSQL enum for `provider`** over a lookup table — compiler-enforced exhaustive matching in Rust, prevents typos, appropriate for a small slowly-changing set.
- **`oauth_connections` is OAuth-specific**, not a generic integrations table — OAuth and API keys have fundamentally different fields/lifecycles. API keys are platform-level for this milestone anyway.
- **No soft-delete** (`is_active`) — row existence is the source of truth. Simpler, no ambiguous state.
- **No speculative columns** (`refresh_count`, `last_refreshed_at`, `error_message`) — `updated_at` covers refresh timing. Add columns when a real use case requires them.
- **Scopes as TEXT** (space-separated per OAuth2 spec) — simpler SeaORM mapping than PostgreSQL arrays.

## Completed Steps

### 1. Entity + enum definitions ✓
Created `Provider` Rust enum (SeaORM `DeriveActiveEnum`), `oauth_connections` entity, added `meeting_url`/`provider` to `coaching_sessions` entity. Removed `meeting_url` and AI privacy columns from `coaching_relationships` entity (no migration existed for those columns).

### 2. Entity API layer ✓
Added CRUD operations for `oauth_connections` (create, find_by_user_and_provider, update_tokens, delete_by_user_and_provider) with tests. Updated `coaching_sessions` create to pass through `meeting_url` and `provider`. Cleaned up `CoachingRelationshipWithUserNames` to match actual schema.

### 3. Domain layer ✓
Created `domain::oauth_connection` module with `google_authorize_url`, `exchange_and_store_tokens`, and `get_valid_access_token` (with automatic token refresh). Added `PlainTokens` and `Tokens::into_plain()` to `meeting-auth` to expose secret tokens at trust boundaries without leaking the `secrecy` crate. Integrated meeting creation into `coaching_session::create` — when a `provider` is specified, it automatically creates a meeting space via the coach's OAuth connection.

### 4. Web layer: OAuth controller ✓
Slimmed `oauth_controller` to thin wrappers calling domain functions. `authorize` → `oauth_connection::google_authorize_url`, `callback` → `oauth_connection::exchange_and_store_tokens`. Removed all inline token handling and error helpers.

### 5. Web layer: session/meeting endpoints (partial) ✓
Meeting creation is integrated into the existing `POST /coaching_sessions` flow — passing a `provider` field triggers automatic meeting space creation. Removed the standalone `POST /coaching_relationships/:id/create-google-meet` endpoint and the `coaching_relationship_controller`.

### 6. Create meeting for existing session ✓
Added `POST /coaching_sessions/:id/meetings` endpoint that creates a provider meeting (e.g. Google Meet) for an existing coaching session. Coach-only, protected by new `UserIsCoach` Check implementation.

**Changes:**
- `entity_api/src/coaching_session.rs` — added `update_meeting()` to set meeting_url and provider
- `domain/src/coaching_session.rs` — added `create_meeting()` orchestrating session lookup → meeting creation → session update
- `web/src/protect/mod.rs` — added `UserIsCoach` Check implementation
- `web/src/protect/coaching_sessions.rs` — added `create_meeting` middleware using `UserCanAccessCoachingSession` + `UserIsCoach`
- `web/src/params/coaching_session/meeting.rs` — new `CreateParams { provider }` (params converted from file to directory module)
- `web/src/controller/coaching_session/meeting_controller.rs` — new `create()` handler
- `web/src/router.rs` — route wired into `coaching_sessions_routes()`

## Remaining Steps

### 7. Remove meeting_recording_controller.rs
- Delete `web/src/controller/meeting_recording_controller.rs`
- Remove module declaration from `web/src/controller/mod.rs`
- Remove routes, OpenAPI paths, and `meeting_recording_routes()` from `web/src/router.rs`
- This controller belongs to the deferred AI/recording milestone and currently has 6 compile errors

### 8. Delete user_integrations and stale references
- Delete `entity/src/user_integrations.rs` entity and `entity_api/src/user_integration.rs` module
- Remove `user_integrations` re-exports from `entity/src/lib.rs`, `entity_api/src/lib.rs`, `domain/src/lib.rs`
- Remove `integration_controller.rs` and its routes from `web/src/router.rs` (or update it to use `oauth_connections`)
- Delete the `user_integrations` params module if it exists
- Search for any remaining `user_integration` references throughout the codebase

### 9. Set existing meeting URL on a session
Verify that `PUT /coaching_sessions/:id` with `meeting_url` in the update params works end-to-end. `meeting_url` is already in `UpdateParams`. Verify readable by both coaches and coachees via session GET responses.

### 10. Final verification
- `cargo check` — full workspace clean
- `cargo clippy` — no warnings
- `cargo test --features mock` — all tests pass
- Manual testing of OAuth flow and meeting creation
