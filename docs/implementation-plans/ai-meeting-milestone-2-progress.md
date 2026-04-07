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

## Phase 2 — Data Access
Step 2 from the plan.

- [x] `entity_api/src/meeting_recording.rs`
- [x] `entity_api/src/transcription.rs`
- [x] `entity_api/src/transcript_segment.rs`

## Phase 3 — Provider + Business Logic
Steps 3, 4 from the plan.

- [ ] `domain/src/gateway/recall_ai/mod.rs`
- [ ] `domain/src/meeting_recording.rs`
- [ ] `domain/src/transcription.rs`

## Phase 4 — Web Layer
Steps 5, 6 from the plan.

- [ ] `SvixValidator` in `meeting-auth/src/webhook/`
- [ ] `web/src/controller/webhook_controller.rs`
- [ ] `web/src/controller/coaching_session/meeting_recording_controller.rs`
- [ ] `web/src/controller/coaching_session/transcription_controller.rs`
- [ ] Route wiring in `web/src/router.rs`
