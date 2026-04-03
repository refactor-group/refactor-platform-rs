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
End-to-end pipeline: coach starts recording → Recall.ai bot joins meeting → recording completes → Recall.ai transcribes with speaker diarization → speaker-labeled transcript segments stored and served to the coaching session conversation UI.

**Key decisions:**
- Recall.ai for recording bots (system-level API key, env var — not per-user)
- Standard async transcription flow: `recording.done` → Recall.ai "Create Async Transcript" (AssemblyAI provider, separate streams) → `transcript.done` → Recall.ai "Retrieve Transcript" → download transcript JSON from `download_url`
- No direct AssemblyAI API calls — Recall.ai is the only external provider; transcript data retrieved entirely through Recall.ai's API
- Speaker labels served as-is from Recall.ai/AssemblyAI; no mapping to user records
- Perfect diarization supported platforms: Zoom, Microsoft Teams, Google Meet (not Webex, Slack Huddles, Go-To Meeting)
- Full webhook automation: each stage triggers the next automatically
- New tables: `meeting_recordings`, `transcriptions`, `transcript_segments`
- `meeting-manager` crate NOT needed (meeting creation already built in existing gateways)
- Recall.ai webhooks use Svix HMAC-SHA256 (needs Svix-specific validator added to `meeting-auth`)
- Transcription lifecycle flows entirely through Recall.ai (`transcript.done` / `transcript.failed`)
- Providers constructed inline at call site — no AppState provider fields needed
- `audio_url` is internal-only — not serialized to API clients (`#[serde(skip_serializing)]`)
- Transcript content lives exclusively in `transcript_segments`; `transcriptions` stores only metadata

---

## Implementation Steps

### Step 1: Database Migrations + SeaORM Entities

**Three new migrations** (follow `migration/src/m20251220_000001_add_oauth_connections.rs` as template):

**`meeting_recordings` table** — recording bot state per coaching session
```sql
id                  UUID PK DEFAULT gen_random_uuid()
coaching_session_id UUID FK → coaching_sessions(id) ON DELETE CASCADE
bot_id              VARCHAR(255) NOT NULL   -- Recall.ai's bot UUID
status              meeting_recording_status ENUM
                    (pending, joining, waiting_room, in_meeting, recording,
                     processing, completed, failed)
video_url           TEXT NULL               -- display URL (serialized to client)
audio_url           TEXT NULL               -- internal only; retained for reference (`#[serde(skip_serializing)]`)
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
external_id          VARCHAR(255) NOT NULL  -- Recall.ai transcript ID (from "Create Async Transcript" response)
status               transcription_status ENUM (queued, processing, completed, failed)
language_code        VARCHAR(20) NULL
speaker_count        SMALLINT NULL
word_count           INTEGER NULL
duration_seconds     INTEGER NULL
confidence           DOUBLE PRECISION NULL
error_message        TEXT NULL
created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
```

> No `text` column — full transcript content lives exclusively in `transcript_segments`. This is the normalized design; the full text can be reconstructed by joining segments ordered by `start_ms`.

**`transcript_segments` table** — speaker-diarized utterances (powers conversation UI)
```sql
id               UUID PK DEFAULT gen_random_uuid()
transcription_id UUID FK → transcriptions(id) ON DELETE CASCADE
speaker_label    VARCHAR(255) NOT NULL   -- display name as returned by AssemblyAI/Recall.ai
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
- `entity/src/meeting_recording.rs` (includes `MeetingRecordingStatus` enum; `audio_url` has `#[serde(skip_serializing)]`)
- `entity/src/transcription.rs` (includes `TranscriptionStatus` enum)
- `entity/src/transcript_segment.rs` (no enums; `sentiment` is `Option<String>`)

> **Critical:** Use `ALTER TYPE refactor_platform.<type_name> OWNER TO refactor` immediately after any `CREATE TYPE`. See `CLAUDE.md` Database Migrations section.

Follow `entity/src/oauth_connections.rs` as the entity model template.

---

### Step 2: Entity API Layer

Pattern: follow `entity_api/src/oauth_connection.rs`.

**`entity_api/src/meeting_recording.rs`**
- `create(db, model) -> Result<Model>`
- `find_latest_by_coaching_session(db, session_id) -> Result<Option<Model>>` — ordered by `created_at DESC`, limit 1; returns the most recent recording
- `find_by_bot_id(db, bot_id) -> Result<Option<Model>>` — used by webhook handler
- `update_status(db, id, status, video_url?, audio_url?, duration_seconds?, started_at?, ended_at?, error_message?) -> Result<Model>`

> Multiple `meeting_recordings` rows may exist per coaching session. Failed and completed recordings are retained as historical records. All callers needing "the current recording" must use `find_latest_by_coaching_session`.

**`entity_api/src/transcription.rs`**
- `create(db, model) -> Result<Model>`
- `find_by_coaching_session(db, session_id) -> Result<Option<Model>>`
- `find_by_external_id(db, external_id) -> Result<Option<Model>>` — used by webhook handler to look up by Recall.ai transcript ID
- `update_status(db, id, status, word_count?, confidence?, error_message?) -> Result<Model>`

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
- `POST /bot` — body includes `meeting_url`, `bot_name`, `webhook_url`, `recording_config`, and `metadata: { "coaching_session_id": "<uuid>" }`
- `POST /recordings/{recording_id}/async-transcripts` — create async transcript after `recording.done`
  - Body: `{ "assemblyai": { "language_detection": true, "sentiment_analysis": true, "speaker_labels": true }, "diarization": { "use_separate_streams_when_available": true } }`
  - Returns a Recall.ai transcript ID; completion signaled via `transcript.done` webhook
- `GET /recordings/{recording_id}/async-transcripts/{transcript_id}` — retrieve transcript metadata + `download_url` after `transcript.done`
- Download the transcript JSON directly from `download_url` (pre-signed URL, no auth header needed)
- `DELETE /bot/{id}` — stop and remove the recording bot

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
- `start(db, recording, config) -> Result<Model>`
  - Constructs `recall_ai::Provider` from config
  - Calls `provider.create_async_transcript(recording_id)` — triggers Recall.ai → AssemblyAI with diarization
  - Persists `transcriptions` row with returned Recall.ai transcript ID as `external_id`, status `queued`
- `handle_completion(db, config, transcript_id) -> Result<()>`
  1. Call `recall_ai::Provider.get_async_transcript(recording_id, transcript_id)` → extract `download_url`
  2. Download transcript JSON from `download_url` (plain HTTPS GET, no auth)
  3. Update `transcriptions` row: `word_count`, `confidence`, `speaker_count`, `duration_seconds`, `status = completed`
  4. Extract utterances → `transcript_segments` via `create_batch`:
     - `speaker_label` = speaker label as returned by Recall.ai/AssemblyAI
     - Map `start/end/confidence/sentiment` directly

**Logging conventions** — use the `log` crate throughout domain processing code:

| Situation | Level | Fields to include |
|-----------|-------|-------------------|
| Processing started (inside spawned task) | `info!` | `session_id` or `external_id` |
| Idempotent skip (duplicate webhook) | `warn!` | entity ID, reason |
| DB lookup failure (sync handler phase) | `error!` | entity ID, error |
| Spawned task failure | `error!` | entity ID, full error chain |
| Invalid webhook signature | `warn!` | provider, header values (no secret values) |
| Processing completed successfully | `info!` | `session_id`, segment count |

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
    → Return 401 Unauthorized on invalid signature (prevents pointless Svix retries)
  - Deserialize event type from JSON body
  - Route:
    - "bot.status_change" → update meeting_recording status from event data
    - "recording.done"    → read coaching_session_id from data.bot.metadata.coaching_session_id
                          → look up meeting_recording by bot_id (return 500 on DB error so Svix retries)
                          → check transcription::find_by_coaching_session — if row already exists, log warn! + return 200 (idempotent)
                          → update meeting_recording status=completed, video_url + audio_url from artifacts, duration_seconds
                          → tokio::spawn { domain::transcription::start(); on Err: error! + update meeting_recording status=failed }
    - "transcript.done"   → read transcript_id from event body
                          → look up transcription by external_id (return 500 on DB error so Svix retries)
                          → if transcription status == completed, log warn! + return 200 (idempotent)
                          → tokio::spawn { domain::transcription::handle_completion(); on Err: error! + update transcription status=failed }
    - "transcript.failed" → look up transcription by external_id
                          → update transcription status=failed, error_message from event
    - "bot.fatal"         → update meeting_recording status=failed, error_message
  - Return 200 OK immediately
```

**Response code semantics:**
- `200 OK` — event received and processing started (or idempotent skip)
- `401 Unauthorized` — signature/auth validation failed; provider should not retry
- `500 Internal Server Error` — DB failure during synchronous lookup; provider should retry

**Retry windows:** Svix retries up to ~24h with exponential backoff.

**Error handling inside `tokio::spawn`:** All spawned tasks must handle their own errors — the webhook has already returned `200 OK`. Pattern:
```rust
tokio::spawn(async move {
    if let Err(e) = domain::transcription::handle_completion(&db, &config, &transcript_id).await {
        error!("Transcription completion failed for transcript_id={}: {:?}", transcript_id, e);
        let _ = transcription::update_status(&db, transcription_id, TranscriptionStatus::Failed,
            None, None, Some(e.to_string())).await;
    }
});
```

---

### Step 6: Web Controllers + Routes

**`web/src/controller/coaching_session/meeting_recording_controller.rs`**

Follow pattern of `web/src/controller/coaching_session/goal_controller.rs`.

```
GET    /coaching_sessions/:id/meeting_recording  → get recording status + artifact URLs
POST   /coaching_sessions/:id/meeting_recording  → create bot + start recording
                                                    → return 409 Conflict if a recording with status
                                                      not in {failed, completed} already exists
                                                      (prevents duplicate active bots)
DELETE /coaching_sessions/:id/meeting_recording  → stop bot
```

**Recovery flow:** If a recording or transcription fails, the coach uses `DELETE` (stop the bot) then `POST` (start a new bot) to restart the full pipeline from the beginning. A new `meeting_recordings` row is created; the failed `transcriptions` row and segments remain as a historical record.

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
- `recall_ai_api_key: SecretString` — env: `RECALL_AI_API_KEY` (used for bot creation and async transcription)
- `recall_ai_region: String` — env: `RECALL_AI_REGION`, default `"us"`
- `recall_ai_webhook_secret: SecretString` — env: `RECALL_AI_WEBHOOK_SECRET` (Svix signing secret)
- `webhook_base_url: String` — env: `WEBHOOK_BASE_URL` (e.g., `https://app.refactorcoach.com`)

> AssemblyAI credentials used by Recall.ai for transcription are configured in the Recall.ai dashboard — no AssemblyAI keys needed in our config.

---

### Step 8: Add to Default Members

Update root `Cargo.toml` `default-members` array to include `"meeting-auth"` and `"meeting-ai"`.

---

## Critical Files

| File | Action |
|------|--------|
| `entity/src/meeting_recording.rs` | Create |
| `entity/src/transcription.rs` | Create |
| `entity/src/transcript_segment.rs` | Create |
| `entity/src/lib.rs` | Update (export new entity modules) |
| `entity_api/src/meeting_recording.rs` | Create |
| `entity_api/src/transcription.rs` | Create |
| `entity_api/src/transcript_segment.rs` | Create |
| `entity_api/src/lib.rs` | Update (export new entity_api modules) |
| `domain/src/meeting_recording.rs` | Create |
| `domain/src/transcription.rs` | Create |
| `domain/src/gateway/recall_ai/mod.rs` | Create (bot creation, async transcript, download) |
| `domain/src/lib.rs` | Update (declare `meeting_recording` and `transcription` modules) |
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
    "external_id": "recall-transcript-id",
    "status": "completed",
    "language_code": "en",
    "speaker_count": 2,
    "word_count": 4200,
    "duration_seconds": 3600,
    "confidence": 0.94,
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
