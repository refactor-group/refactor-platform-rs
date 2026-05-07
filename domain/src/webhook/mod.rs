pub mod bot_fatal;
pub mod bot_status;
pub mod recording_done;
pub mod recording_failed;
pub mod transcript_done;
pub mod transcript_failed;
pub mod transcript_processing;

use crate::error::{DomainErrorKind, Error};
use crate::meeting_recording::MeetingRecordingStatus;
use entity::Id;
use events::EventPublisher;
use log::debug;
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use service::config::Config;
use std::sync::Arc;

// ── Private parsing structs ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct BotField {
    id: String,
}

#[derive(Deserialize)]
struct BotWithMetaField {
    id: String,
    metadata: Option<BotMetadata>,
}

#[derive(Deserialize)]
struct BotMetadata {
    coaching_session_id: Option<String>,
}

#[derive(Deserialize)]
struct RecordingField {
    id: String,
}

#[derive(Deserialize)]
struct TranscriptField {
    id: String,
}

#[derive(Deserialize)]
struct ErrorDetail {
    sub_code: Option<String>,
}

#[derive(Deserialize)]
struct BotStatusData {
    bot: BotField,
}

#[derive(Deserialize)]
struct BotErrorData {
    bot: BotField,
    data: Option<ErrorDetail>,
}

#[derive(Deserialize)]
struct RecordingDoneData {
    bot: BotWithMetaField,
    recording: RecordingField,
}

#[derive(Deserialize)]
struct TranscriptData {
    transcript: TranscriptField,
}

#[derive(Deserialize)]
struct TranscriptErrorData {
    transcript: TranscriptField,
    data: Option<ErrorDetail>,
}

// ── Public event enum ─────────────────────────────────────────────────────────

pub enum Event {
    BotStatus {
        bot_id: String,
        status: MeetingRecordingStatus,
    },
    BotFatal {
        bot_id: String,
        error_message: Option<String>,
    },
    RecordingDone {
        bot_id: String,
        recall_recording_id: String,
        /// None if missing or non-UUID in bot metadata — handler skips gracefully.
        coaching_session_id: Option<Id>,
    },
    RecordingFailed {
        bot_id: String,
        error_message: Option<String>,
    },
    TranscriptDone {
        transcript_id: String,
    },
    TranscriptFailed {
        transcript_id: String,
        error_message: Option<String>,
    },
    TranscriptProcessing {
        transcript_id: String,
    },
    Unknown(String),
}

fn validation_err(msg: impl Into<String>) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Validation(msg.into()),
    }
}

impl Event {
    /// Parse a Recall.ai webhook event type and its data payload into a typed `Event`.
    ///
    /// Returns `Err` (→ 400) only when a required field is missing from the payload.
    /// Unrecognised event types become `Event::Unknown` and are silently accepted (→ 200).
    pub fn parse(event_type: &str, data: serde_json::Value) -> Result<Self, Error> {
        let event = match event_type {
            "bot.joining_call"
            | "bot.in_waiting_room"
            | "bot.in_call_not_recording"
            | "bot.in_call_recording"
            | "bot.done" => {
                let status = match event_type {
                    "bot.joining_call" => MeetingRecordingStatus::Joining,
                    "bot.in_waiting_room" => MeetingRecordingStatus::WaitingRoom,
                    "bot.in_call_not_recording" => MeetingRecordingStatus::InMeeting,
                    "bot.in_call_recording" => MeetingRecordingStatus::Recording,
                    _ => MeetingRecordingStatus::Processing, // bot.done
                };
                let d: BotStatusData = serde_json::from_value(data)
                    .map_err(|e| validation_err(format!("{event_type}: {e}")))?;
                Event::BotStatus {
                    bot_id: d.bot.id,
                    status,
                }
            }
            "bot.fatal" => {
                let d: BotErrorData = serde_json::from_value(data)
                    .map_err(|e| validation_err(format!("bot.fatal: {e}")))?;
                Event::BotFatal {
                    bot_id: d.bot.id,
                    error_message: d.data.and_then(|e| e.sub_code),
                }
            }
            "recording.done" => {
                let d: RecordingDoneData = serde_json::from_value(data)
                    .map_err(|e| validation_err(format!("recording.done: {e}")))?;
                let coaching_session_id = d
                    .bot
                    .metadata
                    .as_ref()
                    .and_then(|m| m.coaching_session_id.as_deref())
                    .and_then(|s| s.parse::<Id>().ok());
                Event::RecordingDone {
                    bot_id: d.bot.id,
                    recall_recording_id: d.recording.id,
                    coaching_session_id,
                }
            }
            "recording.failed" => {
                let d: BotErrorData = serde_json::from_value(data)
                    .map_err(|e| validation_err(format!("recording.failed: {e}")))?;
                Event::RecordingFailed {
                    bot_id: d.bot.id,
                    error_message: d.data.and_then(|e| e.sub_code),
                }
            }
            "transcript.done" => {
                let d: TranscriptData = serde_json::from_value(data)
                    .map_err(|e| validation_err(format!("transcript.done: {e}")))?;
                Event::TranscriptDone {
                    transcript_id: d.transcript.id,
                }
            }
            "transcript.failed" => {
                let d: TranscriptErrorData = serde_json::from_value(data)
                    .map_err(|e| validation_err(format!("transcript.failed: {e}")))?;
                Event::TranscriptFailed {
                    transcript_id: d.transcript.id,
                    error_message: d.data.and_then(|e| e.sub_code),
                }
            }
            "transcript.processing" => {
                let d: TranscriptData = serde_json::from_value(data)
                    .map_err(|e| validation_err(format!("transcript.processing: {e}")))?;
                Event::TranscriptProcessing {
                    transcript_id: d.transcript.id,
                }
            }
            other => Event::Unknown(other.to_string()),
        };

        Ok(event)
    }
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn dispatch(
    db: &Arc<DatabaseConnection>,
    config: &Config,
    event_publisher: &EventPublisher,
    event: Event,
) -> Result<(), Error> {
    match event {
        Event::BotStatus { bot_id, status } => {
            bot_status::handle(db, event_publisher, &bot_id, status).await
        }
        Event::BotFatal {
            bot_id,
            error_message,
        } => bot_fatal::handle(db, event_publisher, &bot_id, error_message).await,
        Event::RecordingDone {
            bot_id,
            recall_recording_id,
            coaching_session_id,
        } => {
            recording_done::handle(
                Arc::clone(db),
                config.clone(),
                event_publisher.clone(),
                &bot_id,
                &recall_recording_id,
                coaching_session_id,
            )
            .await
        }
        Event::RecordingFailed {
            bot_id,
            error_message,
        } => recording_failed::handle(db, event_publisher, &bot_id, error_message).await,
        Event::TranscriptDone { transcript_id } => {
            transcript_done::handle(
                Arc::clone(db),
                config.clone(),
                event_publisher.clone(),
                &transcript_id,
            )
            .await
        }
        Event::TranscriptFailed {
            transcript_id,
            error_message,
        } => transcript_failed::handle(db, event_publisher, &transcript_id, error_message).await,
        Event::TranscriptProcessing { transcript_id } => {
            transcript_processing::handle(&transcript_id);
            Ok(())
        }
        Event::Unknown(name) => {
            debug!("Unhandled Recall.ai webhook event: {}", name);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOT_ID: &str = "bot_abc123";
    const RECORDING_ID: &str = "rec_abc123";
    const TRANSCRIPT_ID: &str = "trans_abc123";
    const SESSION_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn is_validation_err(r: &Result<Event, Error>) -> bool {
        matches!(
            r,
            Err(Error {
                error_kind: DomainErrorKind::Validation(_),
                ..
            })
        )
    }

    // ── Bot status events ─────────────────────────────────────────────────────

    #[test]
    fn bot_status_events_map_to_correct_status() {
        let cases = [
            ("bot.joining_call", MeetingRecordingStatus::Joining),
            ("bot.in_waiting_room", MeetingRecordingStatus::WaitingRoom),
            (
                "bot.in_call_not_recording",
                MeetingRecordingStatus::InMeeting,
            ),
            ("bot.in_call_recording", MeetingRecordingStatus::Recording),
            ("bot.done", MeetingRecordingStatus::Processing),
        ];
        for (event_type, expected_status) in cases {
            let data = json!({ "bot": { "id": BOT_ID } });
            let event = Event::parse(event_type, data).unwrap();
            let Event::BotStatus { bot_id, status } = event else {
                panic!("{event_type}: wrong variant");
            };
            assert_eq!(bot_id, BOT_ID, "{event_type}");
            assert_eq!(status, expected_status, "{event_type}");
        }
    }

    #[test]
    fn bot_status_missing_bot_id_returns_validation_error() {
        let data = json!({ "bot": {} });
        assert!(is_validation_err(&Event::parse("bot.joining_call", data)));
    }

    // ── bot.fatal ─────────────────────────────────────────────────────────────

    #[test]
    fn bot_fatal_with_sub_code() {
        let data = json!({ "bot": { "id": BOT_ID }, "data": { "sub_code": "fatal_error" } });
        let event = Event::parse("bot.fatal", data).unwrap();
        let Event::BotFatal {
            bot_id,
            error_message,
        } = event
        else {
            panic!("wrong variant");
        };
        assert_eq!(bot_id, BOT_ID);
        assert_eq!(error_message.as_deref(), Some("fatal_error"));
    }

    #[test]
    fn bot_fatal_without_data_field() {
        let data = json!({ "bot": { "id": BOT_ID } });
        let event = Event::parse("bot.fatal", data).unwrap();
        let Event::BotFatal { error_message, .. } = event else {
            panic!("wrong variant");
        };
        assert!(error_message.is_none());
    }

    #[test]
    fn bot_fatal_missing_bot_id_returns_validation_error() {
        let data = json!({ "bot": {} });
        assert!(is_validation_err(&Event::parse("bot.fatal", data)));
    }

    // ── recording.done ────────────────────────────────────────────────────────

    #[test]
    fn recording_done_with_valid_session_id() {
        let data = json!({
            "bot": { "id": BOT_ID, "metadata": { "coaching_session_id": SESSION_ID } },
            "recording": { "id": RECORDING_ID }
        });
        let event = Event::parse("recording.done", data).unwrap();
        let Event::RecordingDone {
            bot_id,
            recall_recording_id,
            coaching_session_id,
        } = event
        else {
            panic!("wrong variant");
        };
        assert_eq!(bot_id, BOT_ID);
        assert_eq!(recall_recording_id, RECORDING_ID);
        assert_eq!(coaching_session_id, Some(SESSION_ID.parse::<Id>().unwrap()));
    }

    #[test]
    fn recording_done_missing_coaching_session_id_yields_none() {
        let data = json!({
            "bot": { "id": BOT_ID, "metadata": {} },
            "recording": { "id": RECORDING_ID }
        });
        let event = Event::parse("recording.done", data).unwrap();
        let Event::RecordingDone {
            coaching_session_id,
            ..
        } = event
        else {
            panic!("wrong variant");
        };
        assert!(coaching_session_id.is_none());
    }

    #[test]
    fn recording_done_invalid_coaching_session_id_yields_none() {
        let data = json!({
            "bot": { "id": BOT_ID, "metadata": { "coaching_session_id": "not-a-uuid" } },
            "recording": { "id": RECORDING_ID }
        });
        let event = Event::parse("recording.done", data).unwrap();
        let Event::RecordingDone {
            coaching_session_id,
            ..
        } = event
        else {
            panic!("wrong variant");
        };
        assert!(coaching_session_id.is_none());
    }

    #[test]
    fn recording_done_missing_bot_id_returns_validation_error() {
        let data = json!({
            "bot": { "metadata": { "coaching_session_id": SESSION_ID } },
            "recording": { "id": RECORDING_ID }
        });
        assert!(is_validation_err(&Event::parse("recording.done", data)));
    }

    #[test]
    fn recording_done_missing_recording_id_returns_validation_error() {
        let data = json!({
            "bot": { "id": BOT_ID },
            "recording": {}
        });
        assert!(is_validation_err(&Event::parse("recording.done", data)));
    }

    // ── recording.failed ──────────────────────────────────────────────────────

    #[test]
    fn recording_failed_with_sub_code() {
        let data = json!({ "bot": { "id": BOT_ID }, "data": { "sub_code": "codec_error" } });
        let event = Event::parse("recording.failed", data).unwrap();
        let Event::RecordingFailed { error_message, .. } = event else {
            panic!("wrong variant");
        };
        assert_eq!(error_message.as_deref(), Some("codec_error"));
    }

    #[test]
    fn recording_failed_without_data_field() {
        let data = json!({ "bot": { "id": BOT_ID } });
        let event = Event::parse("recording.failed", data).unwrap();
        let Event::RecordingFailed { error_message, .. } = event else {
            panic!("wrong variant");
        };
        assert!(error_message.is_none());
    }

    // ── transcript.done ───────────────────────────────────────────────────────

    #[test]
    fn transcript_done_parses_correctly() {
        let data = json!({ "transcript": { "id": TRANSCRIPT_ID } });
        let event = Event::parse("transcript.done", data).unwrap();
        let Event::TranscriptDone { transcript_id } = event else {
            panic!("wrong variant");
        };
        assert_eq!(transcript_id, TRANSCRIPT_ID);
    }

    #[test]
    fn transcript_done_missing_transcript_id_returns_validation_error() {
        let data = json!({ "transcript": {} });
        assert!(is_validation_err(&Event::parse("transcript.done", data)));
    }

    // ── transcript.failed ─────────────────────────────────────────────────────

    #[test]
    fn transcript_failed_with_sub_code() {
        let data = json!({
            "transcript": { "id": TRANSCRIPT_ID },
            "data": { "sub_code": "provider_error" }
        });
        let event = Event::parse("transcript.failed", data).unwrap();
        let Event::TranscriptFailed {
            transcript_id,
            error_message,
        } = event
        else {
            panic!("wrong variant");
        };
        assert_eq!(transcript_id, TRANSCRIPT_ID);
        assert_eq!(error_message.as_deref(), Some("provider_error"));
    }

    #[test]
    fn transcript_failed_without_data_field() {
        let data = json!({ "transcript": { "id": TRANSCRIPT_ID } });
        let event = Event::parse("transcript.failed", data).unwrap();
        let Event::TranscriptFailed { error_message, .. } = event else {
            panic!("wrong variant");
        };
        assert!(error_message.is_none());
    }

    // ── transcript.processing ─────────────────────────────────────────────────

    #[test]
    fn transcript_processing_parses_correctly() {
        let data = json!({ "transcript": { "id": TRANSCRIPT_ID } });
        let event = Event::parse("transcript.processing", data).unwrap();
        let Event::TranscriptProcessing { transcript_id } = event else {
            panic!("wrong variant");
        };
        assert_eq!(transcript_id, TRANSCRIPT_ID);
    }

    // ── Unknown events ────────────────────────────────────────────────────────

    #[test]
    fn unknown_event_type_returns_unknown_variant() {
        let event = Event::parse("some.future.event", json!({})).unwrap();
        let Event::Unknown(name) = event else {
            panic!("wrong variant");
        };
        assert_eq!(name, "some.future.event");
    }
}
