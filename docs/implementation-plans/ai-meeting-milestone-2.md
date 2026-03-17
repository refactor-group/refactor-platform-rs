# Meeting AI Abstraction Layer - Milestone 2

**Status:** Implementation Ready
**Date:** 2026-03-17
**Author:** Caleb Bourg & Claude
**Branch:** `ai-transcription-milestone-2`
**Dependencies:** `meeting-auth`, `meeting-ai` (Milestone 1, complete)

## Context

`meeting-ai-abstraction-layer.md` defines a provider-agnostic AI workflow for recording, transcription, and AI analysis of coaching sessions.

**Milestone 1 (complete):**
- `meeting-auth` crate: OAuth, API key auth, HTTP client builder, webhook HMAC validation
- `meeting-ai` crate: provider-agnostic traits + types for recording bots, transcription, analysis
- `coaching_sessions` entity: `meeting_url` + `provider` fields already present
- Meeting creation gateways: `domain/src/gateway/google_meet.rs` + `zoom.rs` functional
- `meeting_auth::credentials::Storage` trait defined but no DB implementation yet

**What this milestone delivers:**
End-to-end pipeline: coach starts recording → Recall.ai bot joins meeting → recording completes → AssemblyAI transcribes → LLM Gateway extracts coaching actions → actions persisted automatically.

**Key decisions:**
- Recall.ai for recording bots (system-level API key, env var — not per-user)
- AssemblyAI for transcription + LLM Gateway analysis (per-user API key, stored encrypted in DB)
- Full webhook automation: each stage triggers the next automatically
- New tables: `api_credentials`, `meeting_recordings`, `transcriptions`
- `meeting-manager` crate NOT needed (meeting creation already built in existing gateways)
- LeMUR is deprecated March 31, 2026 — use AssemblyAI LLM Gateway instead
- Recall.ai webhooks use Svix HMAC-SHA256 (needs Svix-specific validator added to `meeting-auth`)
- AssemblyAI webhooks use custom header auth (not HMAC)
- Providers constructed inline at call site — no AppState provider fields needed

---

## Implementation Steps

### Step 1: Database Migrations + SeaORM Entities

**Three new migrations** (follow `migration/src/m20251220_000001_add_oauth_connections.rs` as template):

**`api_credentials` table** — per-user third-party API keys (AssemblyAI; generic for future providers)
```sql
id           UUID PK DEFAULT gen_random_uuid()
user_id      UUID FK → users(id) ON DELETE CASCADE
provider     VARCHAR(100) NOT NULL       -- "assembly_ai", "recall_ai", etc.
api_key      TEXT NOT NULL               -- encrypted AES-256-GCM (same pattern as oauth_connections)
region       VARCHAR(50) NULL            -- provider-specific (e.g., Recall.ai: "us" or "eu")
config       JSONB NOT NULL DEFAULT '{}'
created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
UNIQUE(user_id, provider)
INDEX(user_id, provider)
```

**`meeting_recordings` table** — recording bot state per coaching session
```sql
id                  UUID PK DEFAULT gen_random_uuid()
coaching_session_id UUID FK → coaching_sessions(id) ON DELETE CASCADE
bot_id              VARCHAR(255) NOT NULL   -- Recall.ai's bot UUID
status              meeting_recording_status ENUM
                    (pending, joining, waiting_room, in_meeting, recording,
                     processing, completed, failed)
video_url           TEXT NULL
audio_url           TEXT NULL
started_at          TIMESTAMPTZ NULL
ended_at            TIMESTAMPTZ NULL
error_message       TEXT NULL
created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
```

**`transcriptions` table** — transcription + analysis state per recording
```sql
id                   UUID PK DEFAULT gen_random_uuid()
coaching_session_id  UUID FK → coaching_sessions(id) ON DELETE CASCADE
meeting_recording_id UUID FK → meeting_recordings(id) ON DELETE CASCADE
external_id          VARCHAR(255) NOT NULL  -- AssemblyAI's transcript ID
status               transcription_status ENUM (queued, processing, completed, failed)
text                 TEXT NULL              -- full transcript text (populated on completion)
language_code        VARCHAR(20) NULL
speaker_count        SMALLINT NULL
duration_seconds     INTEGER NULL
confidence           DOUBLE PRECISION NULL
analysis_completed   BOOLEAN NOT NULL DEFAULT FALSE
error_message        TEXT NULL
created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
```

**New SeaORM entities:**
- `entity/src/api_credentials.rs`
- `entity/src/meeting_recording.rs` (includes `MeetingRecordingStatus` enum)
- `entity/src/transcription.rs` (includes `TranscriptionStatus` enum)

> **Critical:** Use `ALTER TYPE refactor_platform.<type_name> OWNER TO refactor` immediately after any `CREATE TYPE`. See `CLAUDE.md` Database Migrations section.

Follow `entity/src/oauth_connections.rs` as the entity model template.

---

### Step 2: Entity API Layer

Pattern: follow `entity_api/src/oauth_connection.rs`.

**`entity_api/src/api_credentials.rs`**
- `find_by_user_and_provider(db, user_id, provider) -> Result<Option<Model>>`
- `create(db, model) -> Result<Model>`
- `update(db, id, api_key, region, config) -> Result<Model>`
- `delete_by_user_and_provider(db, user_id, provider) -> Result<()>`

**`entity_api/src/meeting_recording.rs`**
- `create(db, model) -> Result<Model>`
- `find_by_coaching_session(db, session_id) -> Result<Option<Model>>`
- `find_by_bot_id(db, bot_id) -> Result<Option<Model>>` — used by webhook handler
- `update_status(db, id, status, video_url?, audio_url?, started_at?, ended_at?, error_message?) -> Result<Model>`

**`entity_api/src/transcription.rs`**
- `create(db, model) -> Result<Model>`
- `find_by_coaching_session(db, session_id) -> Result<Option<Model>>`
- `find_by_external_id(db, external_id) -> Result<Option<Model>>` — used by webhook handler
- `update_status(db, id, status, text?, analysis_completed?, error_message?) -> Result<Model>`

---

### Step 3: Concrete Provider Implementations

**`domain/src/gateway/recall_ai/mod.rs`**

Implements `meeting_ai::traits::recording_bot::Provider`. Constructed from system-level config.

```rust
pub struct Provider {
    client: reqwest::Client,  // Authorization: Token <api_key>
    base_url: String,          // https://api.recall.ai/api/v1
    webhook_url: String,       // e.g. https://app.refactorcoach.com/webhooks/recall_ai
}
```

Key API call:
- `POST /bot` — body includes `meeting_url`, `bot_name`, `webhook_url`, `recording_config`
  (enable `real_time_transcription` and set `destination_url` for bot events)

**`domain/src/gateway/assembly_ai/mod.rs`**

Implements `meeting_ai::traits::transcription::Provider`. Constructed per-request with user's API key.

```rust
pub struct Provider {
    client: reqwest::Client,  // Authorization: <api_key>
    base_url: String,          // https://api.assemblyai.com
    webhook_url: String,       // e.g. https://app.refactorcoach.com/webhooks/assembly_ai
    webhook_auth_header_name: String,   // e.g. "X-Webhook-Secret"
    webhook_auth_header_value: String,  // system-level secret from config
}
```

Key API calls:
- `POST /v2/transcript` — body includes `audio_url`, `webhook_url`, `webhook_auth_header_name`,
  `webhook_auth_header_value`, `speaker_labels: true`
- `GET /v2/transcript/{id}` — fetch completed transcript (called from webhook handler)
- `DELETE /v2/transcript/{id}` — delete transcript data

**Analysis via LLM Gateway** (replaces deprecated LeMUR, which ends March 31, 2026):
- `POST https://llm-gateway.assemblyai.com/v1/chat/completions`
- OpenAI-compatible API; same AssemblyAI key used in `Authorization` header
- Embed transcript text in system message with extraction prompt
- Model configurable via `ASSEMBLY_AI_ANALYSIS_MODEL` env var (default: `claude-sonnet-4-6`)
- Prompt: extract coaching action items as structured JSON with fields: `body`, `assignee`, `due_by`

---

### Step 4: Domain Business Logic

**`domain/src/meeting_recording.rs`**
- `start(db, config, session_id, user_id) -> Result<MeetingRecording>`
  - Constructs `recall_ai::Provider` from config
  - Calls `provider.create_bot(BotConfig { meeting_url, bot_name, webhook_url, ... })`
  - Persists `meeting_recordings` row with returned `bot_id`, status `pending`

**`domain/src/transcription.rs`**
- `start(db, recording, api_key, config) -> Result<Transcription>`
  - Constructs `assembly_ai::Provider` with user's key
  - Calls `provider.create_transcription(TranscriptionConfig { media_url: recording.audio_url, ... })`
  - Persists `transcriptions` row with returned `external_id`, status `queued`
- `handle_completion(db, config, external_id, api_key) -> Result<()>`
  - Fetches full transcript via `GET /v2/transcript/{external_id}`
  - Calls LLM Gateway with transcript text + extraction prompt
  - Parses JSON response → maps to `actions::ActiveModel` → inserts into `actions` table
  - Updates transcription row: `text`, `status = completed`, `analysis_completed = true`

---

### Step 5: Webhook Infrastructure

**Svix validator** — Recall.ai uses Svix HMAC-SHA256, which differs from the existing `HmacValidator`.

Add `SvixValidator` to `meeting-auth/src/webhook/`:
- Signed content format: `{svix-id}.{svix-timestamp}.{raw-body}`
- Secret format: `whsec_<base64-encoded-key>` (base64-decode before use)
- Verify `svix-signature` header (space-delimited, `v1,<base64-sig>` prefix)
- Reject if `svix-timestamp` is older than 5 minutes (replay protection)

**`web/src/controller/webhook_controller.rs`**

```
POST /webhooks/recall_ai
  Headers required: svix-id, svix-timestamp, svix-signature
  - Validate Svix HMAC using RECALL_AI_WEBHOOK_SECRET
  - Deserialize event type from JSON body
  - Route:
    - "bot.done"  → update meeting_recording status=completed, audio_url from artifacts
                  → look up coaching_session → get coach's AssemblyAI key from api_credentials
                  → call domain::transcription::start()
    - "bot.fatal" → update meeting_recording status=failed, error_message
    - other bot.* → update meeting_recording status accordingly
  - Return 200 OK immediately

POST /webhooks/assembly_ai
  - Validate request header X-Webhook-Secret matches ASSEMBLY_AI_WEBHOOK_SECRET
  - Body: { transcript_id, status }
  - Route:
    - status="completed" → look up transcription by external_id
                         → look up user's AssemblyAI key from api_credentials
                         → call domain::transcription::handle_completion()
    - status="error"     → update transcription status=failed, error_message
  - Return 200 OK immediately
```

Both endpoints must return `200 OK` within 15 seconds. Use `tokio::spawn` for longer processing. Svix retries failed deliveries up to ~24h with exponential backoff.

---

### Step 6: Web Controllers + Routes

**`web/src/controller/coaching_session/meeting_recording_controller.rs`**

Follow pattern of `web/src/controller/coaching_session/goal_controller.rs`.

```
POST /coaching_sessions/:id/meeting_recording  → start_recording
GET  /coaching_sessions/:id/meeting_recording  → get recording status + artifact URLs
```

**`web/src/controller/coaching_session/transcription_controller.rs`**

```
GET /coaching_sessions/:id/transcription  → get transcription status + full text + analysis_completed
```

Update `web/src/controller/coaching_session/mod.rs` to declare both new modules.

Add routes to `web/src/router.rs` following existing nested session route patterns.

---

### Step 7: Config Updates

**`service/src/lib.rs`** — add new fields:
- `recall_ai_api_key: SecretString` — env: `RECALL_AI_API_KEY`
- `recall_ai_region: String` — env: `RECALL_AI_REGION`, default `"us"`
- `recall_ai_webhook_secret: SecretString` — env: `RECALL_AI_WEBHOOK_SECRET` (Svix signing secret)
- `assembly_ai_webhook_secret: SecretString` — env: `ASSEMBLY_AI_WEBHOOK_SECRET` (custom header value)
- `webhook_base_url: String` — env: `WEBHOOK_BASE_URL` (e.g., `https://app.refactorcoach.com`)
- `assembly_ai_analysis_model: String` — env: `ASSEMBLY_AI_ANALYSIS_MODEL`, default `"claude-sonnet-4-6"`

---

### Step 8: Add to Default Members

Update root `Cargo.toml` `default-members` array to include `"meeting-auth"` and `"meeting-ai"`.

---

## Critical Files

| File | Action |
|------|--------|
| `entity/src/api_credentials.rs` | Create |
| `entity/src/meeting_recording.rs` | Create |
| `entity/src/transcription.rs` | Create |
| `entity_api/src/api_credentials.rs` | Create |
| `entity_api/src/meeting_recording.rs` | Create |
| `entity_api/src/transcription.rs` | Create |
| `domain/src/meeting_recording.rs` | Create |
| `domain/src/transcription.rs` | Create |
| `domain/src/gateway/recall_ai/mod.rs` | Create |
| `domain/src/gateway/assembly_ai/mod.rs` | Create |
| `meeting-auth/src/webhook/` | Add `SvixValidator` |
| `web/src/controller/coaching_session/meeting_recording_controller.rs` | Create |
| `web/src/controller/coaching_session/transcription_controller.rs` | Create |
| `web/src/controller/coaching_session/mod.rs` | Update (add new modules) |
| `web/src/controller/webhook_controller.rs` | Create |
| `web/src/router.rs` | Add routes |
| `service/src/lib.rs` | Add new config fields |
| `Cargo.toml` | Add meeting-ai + meeting-auth to default-members |

**Reference files (patterns to follow, do not modify):**
- `entity/src/oauth_connections.rs` — entity model + PostgreSQL enum pattern
- `entity_api/src/oauth_connection.rs` — entity API CRUD pattern
- `domain/src/oauth_token_storage.rs` — AES-256-GCM encryption/decryption pattern
- `web/src/controller/coaching_session/goal_controller.rs` — nested controller pattern
- `meeting-auth/src/webhook/` — existing HMAC validator (adapt for Svix format)

---

## Verification

1. `cargo build` — compiles clean with meeting-ai + meeting-auth in default-members
2. `cargo clippy` — no warnings
3. `cargo fmt` — no formatting changes
4. Run migrations against local DB — verify three new tables exist with correct schema
5. `POST /coaching_sessions/:id/meeting_recording` → confirm `meeting_recordings` row created with `bot_id`, `status=pending`
6. Send test Recall.ai `bot.done` webhook payload → confirm `meeting_recordings` status updated, `transcriptions` row created
7. Send test AssemblyAI `completed` webhook → confirm transcript text stored, actions extracted and inserted into `actions` table, `analysis_completed=true`
8. `GET /coaching_sessions/:id/transcription` → returns transcript text and `analysis_completed: true`
