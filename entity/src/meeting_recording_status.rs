use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Status of a meeting recording through its lifecycle.
#[derive(
    Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Default, Serialize, DeriveActiveEnum,
)]
#[serde(rename_all = "lowercase")]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "meeting_recording_status"
)]
pub enum MeetingRecordingStatus {
    /// Recording has been requested but bot hasn't joined yet
    #[sea_orm(string_value = "pending")]
    #[default]
    Pending,
    /// Bot is joining the meeting
    #[sea_orm(string_value = "joining")]
    Joining,
    /// Actively recording the meeting
    #[sea_orm(string_value = "recording")]
    Recording,
    /// Recording complete, processing/uploading
    #[sea_orm(string_value = "processing")]
    Processing,
    /// Recording fully complete and available
    #[sea_orm(string_value = "completed")]
    Completed,
    /// Recording failed at some stage
    #[sea_orm(string_value = "failed")]
    Failed,
}

impl std::fmt::Display for MeetingRecordingStatus {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeetingRecordingStatus::Pending => write!(fmt, "pending"),
            MeetingRecordingStatus::Joining => write!(fmt, "joining"),
            MeetingRecordingStatus::Recording => write!(fmt, "recording"),
            MeetingRecordingStatus::Processing => write!(fmt, "processing"),
            MeetingRecordingStatus::Completed => write!(fmt, "completed"),
            MeetingRecordingStatus::Failed => write!(fmt, "failed"),
        }
    }
}
