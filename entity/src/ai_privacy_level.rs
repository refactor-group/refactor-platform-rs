use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Per-relationship privacy setting for AI features.
/// Allows coaches to configure AI integration on a per-client basis.
#[derive(
    Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Default, Serialize, DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "ai_privacy_level")]
pub enum AiPrivacyLevel {
    /// No AI recording or transcribing - for clients uncomfortable with AI
    #[sea_orm(string_value = "none")]
    None,
    /// Text transcription only, no video/audio storage
    #[sea_orm(string_value = "transcribe_only")]
    TranscribeOnly,
    /// All AI recording and transcribing features enabled
    #[sea_orm(string_value = "full")]
    #[default]
    Full,
}

impl std::fmt::Display for AiPrivacyLevel {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiPrivacyLevel::None => write!(fmt, "none"),
            AiPrivacyLevel::TranscribeOnly => write!(fmt, "transcribe_only"),
            AiPrivacyLevel::Full => write!(fmt, "full"),
        }
    }
}
