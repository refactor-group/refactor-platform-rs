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

## Remaining Steps

### 1. Entity + enum definitions
Create `Provider` Rust enum (SeaORM `DeriveActiveEnum`), `oauth_connections` entity, add `meeting_url`/`provider` to `coaching_sessions` entity. Remove `meeting_url` and AI privacy columns from `coaching_relationships` entity. Delete `user_integrations` entity.

### 2. Entity API layer
Add CRUD operations for `oauth_connections` (store, get, update, delete tokens by user+provider). Update `coaching_sessions` create/update to accept `meeting_url` and `provider`.

### 3. Domain layer
Wire the existing `meeting-auth` Google OAuth provider and `google_meet::Client` to use `oauth_connections` storage instead of `user_integrations`. Token refresh logic should use the stored `refresh_token` from `oauth_connections`.

### 4. Web layer: OAuth controller
Update `authorize` and `callback` endpoints to read/write `oauth_connections` instead of `user_integrations`. Store encrypted tokens via `domain::encryption`.

### 5. Web layer: session/meeting endpoints
Move `create_google_meet` from coaching relationship controller to coaching session context. Add endpoint to set an existing meeting URL on a session. Both coaches and coachees need to read the `meeting_url`.

### 6. Cleanup
Delete `user_integrations` entity and all references throughout the codebase. Fix `meeting_ai` import errors in `meeting_recording_controller.rs`. Remove stale fields from `coaching_relationships` entity.
