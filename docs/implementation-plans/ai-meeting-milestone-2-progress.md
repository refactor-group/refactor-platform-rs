# Milestone 2 Progress

See `ai-meeting-milestone-2.md` for full detail on each step.

## Phase 1 — Foundation ✅
Steps 8, 7, 1 from the plan.

- [x] `meeting-ai` + `meeting-auth` added to `default-members` in root `Cargo.toml`
- [x] Config fields added to `service/src/config.rs`: `recall_ai_api_key`, `recall_ai_region`, `recall_ai_webhook_secret`, `webhook_base_url`
- [x] Migration: `m20260407_000001_add_meeting_recordings` (enum + table)
- [x] Migration: `m20260407_000002_add_transcriptions` (enum + table)
- [x] Migration: `m20260407_000003_add_transcript_segments` (table)
- [x] Entity: `entity/src/meeting_recording.rs`
- [x] Entity: `entity/src/transcription.rs`
- [x] Entity: `entity/src/transcript_segment.rs`

## Phase 2 — Data Access ✅
Step 2 from the plan.

- [x] `entity_api/src/meeting_recording.rs`
- [x] `entity_api/src/transcription.rs`
- [x] `entity_api/src/transcript_segment.rs`

## Phase 3 — Provider + Business Logic ✅
Steps 3, 4 from the plan.

- [x] `domain/src/gateway/recall_ai/mod.rs`
- [x] `domain/src/meeting_recording.rs`
- [x] `domain/src/transcription.rs`
- [x] `domain/src/transcript_segment.rs` (re-exports for web layer)

## Phase 4 — Web Layer ✅
Steps 5, 6 from the plan.

- [x] `SvixValidator` in `meeting-auth/src/webhook/svix.rs`
- [x] `web/src/controller/webhook_controller.rs`
- [x] `web/src/controller/coaching_session/meeting_recording_controller.rs`
- [x] `web/src/controller/coaching_session/transcription_controller.rs`
- [x] `web/src/controller/coaching_session/transcription_segment_controller.rs`
- [x] Route wiring in `web/src/router.rs`

## Notes / Deviations from Plan

- Segments controller moved to its own file at `/coaching_sessions/:id/transcriptions/:transcription_id/transcription_segments` (plan had it nested under `/transcription/segments`)
- `/transcription` endpoint pluralized to `/transcriptions` to match project convention
- `WebErrorKind::Conflict` added to `web/src/error.rs` for the 409 duplicate-bot guard
- `entity` added as direct dependency to `domain/Cargo.toml` so domain modules can re-export entity model types to the web layer
