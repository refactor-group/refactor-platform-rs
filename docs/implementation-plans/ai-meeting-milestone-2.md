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
End-to-end pipeline: coach starts recording → Recall.ai bot joins meeting → recording completes → AssemblyAI transcribes → LLM Gateway extracts coaching actions → actions persisted automatically. Speaker-diarized transcript segments stored and served for the coaching session conversation UI.

**Key decisions:**
- Recall.ai for recording bots (system-level API key, env var — not per-user)
- AssemblyAI for transcription + LLM Gateway analysis (per-user API key, stored encrypted in DB)
- Full webhook automation: each stage triggers the next automatically
- New tables: `api_credentials`, `meeting_recordings`, `transcriptions`, `transcript_segments`
- `meeting-manager` crate NOT needed (meeting creation already built in existing gateways)
- LeMUR is deprecated March 31, 2026 — use AssemblyAI LLM Gateway instead
- Recall.ai webhooks use Svix HMAC-SHA256 (needs Svix-specific validator added to `meeting-auth`)
- AssemblyAI webhooks use custom header auth (not HMAC)
- Providers constructed inline at call site — no AppState provider fields needed
- `audio_url` is internal-only — not serialized to API clients (`#[serde(skip_serializing)]`)
- Transcript content lives exclusively in `transcript_segments`; `transcriptions` stores only metadata
- Speaker labels resolved to real user names via the coaching relationship at segment creation time

---

## Implementation Steps

### Step 1: Database Migrations + SeaORM Entities

**Four new migrations** (follow `migration/src/m20251220_000001_add_oauth_connections.rs` as template):

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
video_url           TEXT NULL               -- display URL (serialized to client)
audio_url           TEXT NULL               -- internal only; used to submit to AssemblyAI
duration_seconds    INTEGER NULL
started_at          TIMESTAMPTZ NULL
ended_at            TIMESTAMPTZ NULL
error_message       TEXT NULL
created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
```

**`transcriptions` table** — transcription metadata per recording
```sql
id                   UUID PK DEFAULT gen_random_uuid()
coaching_session_id  UUID FK → coaching_sessions(id) ON DELETE CASCADE
meeting_recording_id UUID FK → meeting_recordings(id) ON DELETE CASCADE
external_id          VARCHAR(255) NOT NULL  -- AssemblyAI's transcript ID
status               transcription_status ENUM (queued, processing, completed, failed)
summary              TEXT NULL              -- LLM-generated coaching summary
language_code        VARCHAR(20) NULL
speaker_count        SMALLINT NULL
word_count           INTEGER NULL
duration_seconds     INTEGER NULL
confidence           DOUBLE PRECISION NULL
analysis_completed   BOOLEAN NOT NULL DEFAULT FALSE
error_message        TEXT NULL
created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
```

> No `text` column — full transcript content lives exclusively in `transcript_segments`. This is the normalized design; the full text can be reconstructed by joining segments ordered by `start_ms`.

**`transcript_segments` table** — speaker-diarized utterances (powers conversation UI)
```sql
id               UUID PK DEFAULT gen_random_uuid()
transcription_id UUID FK → transcriptions(id) ON DELETE CASCADE
speaker_label    VARCHAR(255) NOT NULL   -- resolved user display name; "Speaker A" fallback
speaker_user_id  UUID FK → users(id) ON DELETE SET NULL NULL
text             TEXT NOT NULL
start_ms         INTEGER NOT NULL
end_ms           INTEGER NOT NULL
confidence       DOUBLE PRECISION NULL
sentiment        VARCHAR(20) NULL        -- "positive", "neutral", "negative" (VARCHAR, not enum)
created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
INDEX(transcription_id, start_ms)       -- supports ordered fetch
```

> No `updated_at` — segments are immutable once written. Sentiment stored as VARCHAR to avoid PostgreSQL enum ownership friction.

**New SeaORM entities:**
- `entity/src/api_credentials.rs`
- `entity/src/meeting_recording.rs` (includes `MeetingRecordingStatus` enum; `audio_url` has `#[serde(skip_serializing)]`)
- `entity/src/transcription.rs` (includes `TranscriptionStatus` enum)
- `entity/src/transcript_segment.rs` (no enums; `sentiment` is `Option<String>`)

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
- `update_status(db, id, status, video_url?, audio_url?, duration_seconds?, started_at?, ended_at?, error_message?) -> Result<Model>`

**`entity_api/src/transcription.rs`**
- `create(db, model) -> Result<Model>`
- `find_by_coaching_session(db, session_id) -> Result<Option<Model>>`
- `find_by_external_id(db, external_id) -> Result<Option<Model>>` — used by webhook handler
- `update_status(db, id, status, summary?, word_count?, confidence?, analysis_completed?, error_message?) -> Result<Model>`

**`entity_api/src/transcript_segment.rs`**
- `create_batch(db, segments: Vec<ActiveModel>) -> Result<Vec<Model>>`
- `find_by_transcription(db, transcription_id) -> Result<Vec<Model>>` — ordered by `start_ms ASC`

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

Key API calls:
- `POST /bot` — body includes `meeting_url`, `bot_name`, `webhook_url`, `recording_config`
- `DELETE /bot/{id}` — stop and remove the recording bot

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
  `webhook_auth_header_value`, `speaker_labels: true`, `sentiment_analysis: true`
- `GET /v2/transcript/{id}` — fetch completed transcript (called from webhook handler)
- `DELETE /v2/transcript/{id}` — delete transcript data

**Analysis via LLM Gateway** (replaces deprecated LeMUR, which ends March 31, 2026):
- `POST https://llm-gateway.assemblyai.com/v1/chat/completions`
- OpenAI-compatible API; same AssemblyAI key used in `Authorization` header
- Input: full transcript text reconstructed by concatenating `transcript_segments.text` ordered by `start_ms`
- Model configurable via `ASSEMBLY_AI_ANALYSIS_MODEL` env var (default: `claude-sonnet-4-6`)
- Prompt: extract coaching action items as structured JSON with fields: `body`, `assignee`, `due_by`

---

### Step 4: Domain Business Logic

**`domain/src/meeting_recording.rs`**
- `start(db, config, session_id) -> Result<Model>`
  - Constructs `recall_ai::Provider` from config
  - Calls `provider.create_bot(BotConfig { meeting_url, bot_name, webhook_url, ... })`
  - Persists `meeting_recordings` row with returned `bot_id`, status `pending`
- `stop(db, config, bot_id) -> Result<Model>`
  - Calls `provider.delete_bot(bot_id)` (Recall.ai `DELETE /bot/{id}`)
  - Updates `meeting_recordings` status accordingly

**`domain/src/transcription.rs`**
- `start(db, recording, api_key, config) -> Result<Model>`
  - Constructs `assembly_ai::Provider` with user's key
  - Calls `provider.create_transcription(TranscriptionConfig { media_url: recording.audio_url, ... })`
  - Persists `transcriptions` row with returned `external_id`, status `queued`
- `handle_completion(db, config, external_id, api_key) -> Result<()>`
  1. Fetch full transcript JSON via `GET /v2/transcript/{external_id}`
  2. Update `transcriptions` row: `word_count`, `confidence`, `speaker_count`, `duration_seconds`, `status = completed`
  3. **Speaker resolution**: look up `coaching_session` → `coaching_relationship` → fetch coach and coachee user records from `users` table
     - Collect distinct AssemblyAI speaker labels from utterances, sort alphabetically
     - Map label index 0 → coach user, index 1 → coachee user (deterministic mapping)
     - Prefer `display_name`, fall back to `"first_name last_name"`, fall back to `"Speaker A"` etc.
  4. Extract `utterances` → `transcript_segments` via `create_batch`:
     - `speaker_label` = resolved user name (or fallback)
     - `speaker_user_id` = resolved user ID (or NULL if fallback)
     - Map `start/end/confidence/sentiment` directly
  5. Reconstruct full text for LLM: query `transcript_segments` ordered by `start_ms`, concatenate with newlines
  6. Call LLM Gateway with concatenated text + extraction prompt
  7. Parse JSON response → map to `actions::ActiveModel` → insert into `actions` table
  8. Update `transcriptions` row: `summary` = LLM output, `analysis_completed = true`

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
    - "bot.done"  → update meeting_recording status=completed, video_url + audio_url from artifacts, duration_seconds
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
                         → call domain::transcription::handle_completion() via tokio::spawn
    - status="error"     → update transcription status=failed, error_message
  - Return 200 OK immediately
```

Both endpoints must return `200 OK` within 15 seconds. Use `tokio::spawn` for longer processing. Svix retries failed deliveries up to ~24h with exponential backoff.

---

### Step 6: Web Controllers + Routes

**`web/src/controller/coaching_session/meeting_recording_controller.rs`**

Follow pattern of `web/src/controller/coaching_session/goal_controller.rs`.

```
GET    /coaching_sessions/:id/meeting_recording  → get recording status + artifact URLs
POST   /coaching_sessions/:id/meeting_recording  → create bot + start recording
DELETE /coaching_sessions/:id/meeting_recording  → stop bot
```

**`web/src/controller/coaching_session/transcription_controller.rs`**

```
GET /coaching_sessions/:id/transcription          → get transcription metadata + status
GET /coaching_sessions/:id/transcription/segments → get ordered speaker turns (conversation UI)
```

Update `web/src/controller/coaching_session/mod.rs` to declare both new modules.

Add route functions to `web/src/router.rs`:
```rust
fn coaching_session_meeting_recording_routes(app_state: AppState) -> Router { ... }
fn coaching_session_transcription_routes(app_state: AppState) -> Router { ... }
```
Merge both into `define_routes()` following existing nested session route patterns.

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
| `entity/src/transcript_segment.rs` | Create |
| `entity_api/src/api_credentials.rs` | Create |
| `entity_api/src/meeting_recording.rs` | Create |
| `entity_api/src/transcription.rs` | Create |
| `entity_api/src/transcript_segment.rs` | Create |
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

## API Contract

### `GET /coaching_sessions/:id/meeting_recording`
```json
{
  "status_code": 200,
  "data": {
    "id": "uuid",
    "coaching_session_id": "uuid",
    "bot_id": "recall-bot-uuid",
    "status": "recording",
    "video_url": "https://...",
    "duration_seconds": 3600,
    "started_at": "2026-03-17T10:00:00Z",
    "ended_at": null,
    "error_message": null,
    "created_at": "2026-03-17T10:00:00Z",
    "updated_at": "2026-03-17T10:00:00Z"
  }
}
```
> `audio_url` is never serialized (`#[serde(skip_serializing)]`). Status set: `pending, joining, waiting_room, in_meeting, recording, processing, completed, failed`.

### `GET /coaching_sessions/:id/transcription`
```json
{
  "status_code": 200,
  "data": {
    "id": "uuid",
    "coaching_session_id": "uuid",
    "meeting_recording_id": "uuid",
    "external_id": "assemblyai-transcript-id",
    "status": "completed",
    "summary": "LLM-generated coaching summary...",
    "language_code": "en",
    "speaker_count": 2,
    "word_count": 4200,
    "duration_seconds": 3600,
    "confidence": 0.94,
    "analysis_completed": true,
    "error_message": null,
    "created_at": "...",
    "updated_at": "..."
  }
}
```
> No `text` field — full transcript content lives in segments only.

### `GET /coaching_sessions/:id/transcription/segments`
```json
{
  "status_code": 200,
  "data": [
    {
      "id": "uuid",
      "transcription_id": "uuid",
      "speaker_label": "Jane Smith",
      "speaker_user_id": "coach-user-uuid",
      "text": "What goals are you working toward this quarter?",
      "start_ms": 1000,
      "end_ms": 5200,
      "confidence": 0.97,
      "sentiment": "neutral",
      "created_at": "..."
    },
    {
      "id": "uuid",
      "transcription_id": "uuid",
      "speaker_label": "Alex Johnson",
      "speaker_user_id": "coachee-user-uuid",
      "text": "I want to finish the product roadmap and improve team velocity.",
      "start_ms": 5800,
      "end_ms": 11000,
      "confidence": 0.95,
      "sentiment": "positive",
      "created_at": "..."
    }
  ]
}
```

---

## Verification

1. `cargo build` — compiles clean with meeting-ai + meeting-auth in default-members
2. `cargo clippy` — no warnings
3. `cargo fmt` — no formatting changes
4. Run migrations → verify four new tables with correct schema; `transcript_segments` has composite index `(transcription_id, start_ms)` and no `updated_at`
5. `POST /coaching_sessions/:id/meeting_recording` → `meeting_recordings` row with `bot_id`, `status=pending`; response contains no `audio_url`
6. `DELETE /coaching_sessions/:id/meeting_recording` → bot stopped, status updated
7. Send test Recall.ai `bot.done` webhook → `meeting_recordings` updated with `video_url` (internal `audio_url` stored but not returned), `transcriptions` row created
8. Send test AssemblyAI `completed` webhook → `transcriptions` updated with `word_count`, `confidence` (no `text` column); `transcript_segments` rows inserted ordered by `start_ms`; `speaker_label` values contain resolved user names (not "Speaker A/B"); `speaker_user_id` populated
9. `GET /coaching_sessions/:id/transcription/segments` → segments ordered by `start_ms`, real user names in `speaker_label`
10. `GET /coaching_sessions/:id/transcription` → no `text` field, `analysis_completed: true`, `summary` populated, actions visible in `actions` table
