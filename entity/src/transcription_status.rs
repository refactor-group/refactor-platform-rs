use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Status of a transcription through its lifecycle.
#[derive(
    Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Default, Serialize, DeriveActiveEnum,
)]
#[serde(rename_all = "lowercase")]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "transcription_status"
)]
pub enum TranscriptionStatus {
    /// Transcription has been requested but not started
    #[sea_orm(string_value = "pending")]
    #[default]
    Pending,
    /// Transcription is being processed by AssemblyAI
    #[sea_orm(string_value = "processing")]
    Processing,
    /// Transcription complete and available
    #[sea_orm(string_value = "completed")]
    Completed,
    /// Transcription failed
    #[sea_orm(string_value = "failed")]
    Failed,
}

impl std::fmt::Display for TranscriptionStatus {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptionStatus::Pending => write!(fmt, "pending"),
            TranscriptionStatus::Processing => write!(fmt, "processing"),
            TranscriptionStatus::Completed => write!(fmt, "completed"),
            TranscriptionStatus::Failed => write!(fmt, "failed"),
        }
    }
}
