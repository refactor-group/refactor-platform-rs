//! Types for recording bot operations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lifecycle status of a recording bot joining and recording a meeting.
///
/// Bots transition through states from Pending → Joining → InMeeting → Recording → Completed.
/// Failed status may occur at any point due to auth issues, meeting not found, or bot rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Joining,
    WaitingRoom,
    InMeeting,
    Recording,
    Processing,
    Completed,
    Failed,
}

/// Media artifacts produced by a recording bot after meeting ends.
///
/// URLs typically expire after 24-48 hours, so download and persist files
/// or trigger transcription immediately. All URLs are pre-signed for direct download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifacts {
    pub video_url: Option<String>,
    pub audio_url: Option<String>,
    pub duration_seconds: Option<i32>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub file_size_bytes: Option<u64>,
    pub metadata: HashMap<String, String>,
}

/// Historical record of bot status transitions.
///
/// Useful for debugging, analytics, and understanding bot lifecycle.
/// Providers send these via webhooks or return in get_bot_status calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusChange {
    pub status: Status,
    pub timestamp: DateTime<Utc>,
    pub message: Option<String>,
}

/// Complete information about a recording bot's state and outputs.
///
/// Monitor status field and artifacts become available when status reaches Completed.
/// Check error_message when status is Failed to diagnose issues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Info {
    pub id: String,
    pub meeting_url: String,
    pub status: Status,
    pub artifacts: Option<Artifacts>,
    pub error_message: Option<String>,
    pub status_history: Vec<StatusChange>,
}

/// Configuration for deploying a recording bot to a meeting.
///
/// Provider-specific options (e.g., video quality, streaming endpoints) go in provider_options.
/// Set webhook_url to receive async status updates; without it, you must poll get_bot_status.
#[derive(Debug, Clone)]
pub struct Config {
    pub meeting_url: String,
    pub bot_name: String,
    pub webhook_url: Option<String>,
    pub record_video: bool,
    pub record_audio: bool,
    pub enable_realtime_transcription: bool,
    pub provider_options: HashMap<String, String>,
}

/// Optional filters for listing bots when querying bot history.
///
/// Useful for debugging, showing user's bot history, or finding active bots.
/// Unset fields are not applied as filters (returns all matches).
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub status: Option<Status>,
    pub meeting_url: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
}
