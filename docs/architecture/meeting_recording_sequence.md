# Meeting Recording & Transcription Sequence

Full lifecycle from coach starting a recording through transcript segments being rendered in the UI.

**Webhook delivery note:** All events (`bot.*`, `recording.*`, `transcript.*`) are delivered via the account-level subscription configured in the Recall.ai dashboard to `POST /webhooks/recall_ai`. Each request is verified using Svix HMAC-SHA256 signature validation with a 5-minute replay window.

```mermaid
sequenceDiagram
    participant FE as Next.js (FE)
    participant BE as Axum Backend
    participant DB as PostgreSQL
    participant Recall as Recall.ai

    rect rgb(220, 235, 255)
        Note over FE,Recall: Phase 1 — Start Recording
        FE->>BE: POST /coaching_sessions/:id/meeting_recording<br/>{ meeting_url }
        BE->>DB: find_latest_by_coaching_session (conflict check)
        DB-->>BE: None (or Completed/Failed — safe to proceed)
        BE->>Recall: POST /api/v1/bot/<br/>{ meeting_url, bot_name,<br/>metadata: { coaching_session_id } }
        Recall-->>BE: { id: bot_id }
        BE->>DB: INSERT meeting_recordings (status: Pending, bot_id)
        DB-->>BE: recording row
        BE-->>FE: 201 { status: "pending", bot_id, ... }
        FE->>FE: mutate SWR cache, begin polling every 5s
    end

    rect rgb(220, 255, 220)
        Note over FE,Recall: Phase 2 — Bot Joins Meeting
        Recall->>BE: POST /webhooks/recall_ai<br/>{ event: "bot.joining_call", data: { bot: { id } } }
        BE->>DB: update_status → Joining
        Recall->>BE: POST /webhooks/recall_ai { event: "bot.in_waiting_room" }
        BE->>DB: update_status → WaitingRoom
        Recall->>BE: POST /webhooks/recall_ai { event: "bot.in_call_not_recording" }
        BE->>DB: update_status → InMeeting
        Recall->>BE: POST /webhooks/recall_ai { event: "bot.in_call_recording" }
        BE->>DB: update_status → Recording
    end

    rect rgb(255, 255, 220)
        Note over FE,BE: Phase 3 — FE Polls During Active Recording (every 5s)
        loop While status is Joining / WaitingRoom / InMeeting / Recording / Processing
            FE->>BE: GET /coaching_sessions/:id/meeting_recording
            BE->>DB: find_latest_by_coaching_session
            DB-->>BE: recording
            BE-->>FE: { status: "recording", ... }
        end
        Note over FE: Polling pauses when browser tab is hidden (Page Visibility API)
    end

    rect rgb(255, 235, 220)
        Note over FE,Recall: Phase 4 — Stop Recording (user-initiated)
        FE->>BE: DELETE /coaching_sessions/:id/meeting_recording
        BE->>Recall: POST /api/v1/bot/:bot_id/leave_call/
        Recall-->>BE: 200 (4xx also treated as success — bot may have already left)
        BE->>DB: update_status → Processing, set ended_at
        DB-->>BE: recording
        BE-->>FE: 200 { status: "processing" }
        FE->>FE: mutate SWR cache
    end

    rect rgb(235, 220, 255)
        Note over BE,Recall: Phase 5 — Bot Shutdown
        Recall->>BE: POST /webhooks/recall_ai { event: "bot.done",<br/>data: { bot: { id } } }
        BE->>DB: update_status → Processing (idempotent — already Processing; skipped if terminal)
    end

    rect rgb(240, 220, 255)
        Note over FE,Recall: Phase 6 — Recording Artifact Ready
        Recall->>BE: POST /webhooks/recall_ai<br/>{ event: "recording.done",<br/>data: { bot: { id, metadata: { coaching_session_id } },<br/>recording: { id: recall_recording_id } } }
        BE->>DB: find_by_bot_id
        DB-->>BE: recording
        BE->>DB: find_by_coaching_session (idempotency: transcription exists?)
        DB-->>BE: None
        BE->>DB: update_status → Completed, set ended_at
        Note over BE: Async task spawned (tokio::spawn); 200 returned immediately
        BE-->>Recall: 200 (webhook acknowledged)
        BE->>Recall: POST /api/v1/recording/:recall_recording_id/create_transcript/<br/>{ provider: { assembly_ai_async: { language_detection: true,<br/>sentiment_analysis: true, speaker_labels: true } },<br/>diarization: { use_separate_streams_when_available: true } }
        Recall-->>BE: { id: transcript_id }
        BE->>DB: INSERT transcriptions (status: Queued, external_id: transcript_id,<br/>recall_recording_id)
    end

    rect rgb(255, 220, 235)
        Note over FE,Recall: Phase 7 — Transcription Processing
        FE->>BE: GET /coaching_sessions/:id/transcriptions
        BE->>DB: find_by_coaching_session
        DB-->>BE: transcription (status: Queued)
        BE-->>FE: { status: "queued" }
        FE->>FE: Begin polling every 10s (30 min timeout)

        Note over Recall: Recall.ai/AssemblyAI processes audio

        Recall->>BE: POST /webhooks/recall_ai<br/>{ event: "transcript.processing",<br/>data: { transcript: { id } } }
        BE-->>Recall: 200 (acknowledged; debug log only)

        Recall->>BE: POST /webhooks/recall_ai<br/>{ event: "transcript.done",<br/>data: { transcript: { id } } }
        BE->>DB: find_by_external_id (→ 500/Svix retry if not found)
        DB-->>BE: transcription (status: Queued)
        BE->>DB: try_claim_for_processing (atomic UPDATE WHERE status='queued')
        DB-->>BE: claimed = true
        Note over BE: Async task spawned (tokio::spawn); 200 returned immediately
        BE-->>Recall: 200 (webhook acknowledged)
        BE->>Recall: GET /api/v1/transcript/:transcript_id/
        Recall-->>BE: { id, status, data: { download_url } }
        BE->>Recall: GET {download_url} (pre-signed S3 URL, no auth)
        Recall-->>BE: transcript JSON [{ participant: { name, id },<br/>words: [{ text, start_timestamp: { relative },<br/>end_timestamp: { relative } }] }]
        Note over BE: Flatten words by speaker → sort chronologically →<br/>coalesce (same speaker + gap < 1.5s = same segment)
        BE->>DB: update_status → Completed, set word_count
        BE->>DB: batch INSERT transcript_segments<br/>(speaker_label, text, start_ms, end_ms)
    end

    rect rgb(220, 255, 245)
        Note over FE,BE: Phase 8 — FE Renders Transcript
        FE->>BE: GET /coaching_sessions/:id/transcriptions
        BE->>DB: find_by_coaching_session
        DB-->>BE: transcription (status: Completed)
        BE-->>FE: { status: "completed", id }
        FE->>FE: Stop polling (terminal status)
        FE->>BE: GET /coaching_sessions/:id/transcriptions/:id/transcription_segments
        BE->>DB: list segments for transcription_id ORDER BY start_ms ASC
        DB-->>BE: [{ speaker_label, text, start_ms, end_ms }]
        BE-->>FE: segments array
        FE->>FE: Render transcript (grouped bubbles, search, speaker filter)
    end

    rect rgb(255, 220, 220)
        Note over FE,Recall: Error Paths
        Note over Recall,BE: bot.fatal → update recording → Failed (data.sub_code as error_message)
        Note over Recall,BE: recording.failed → update recording → Failed (data.sub_code as error_message)
        Note over Recall,BE: transcript.failed → update transcription → Failed (data.sub_code as error_message)
        Note over Recall,BE: transcript.done with no matching DB record → 500 (Svix retries delivery)
        Note over FE,BE: FE polls until Failed status, then stops and shows error UI
    end
```
