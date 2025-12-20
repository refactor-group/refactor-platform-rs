use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Type of AI-suggested item extracted from transcription.
#[derive(Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "ai_suggestion_type")]
pub enum AiSuggestionType {
    /// Action item extracted from conversation
    #[sea_orm(string_value = "action")]
    Action,
    /// Agreement extracted from conversation
    #[sea_orm(string_value = "agreement")]
    Agreement,
}

impl std::fmt::Display for AiSuggestionType {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiSuggestionType::Action => write!(fmt, "action"),
            AiSuggestionType::Agreement => write!(fmt, "agreement"),
        }
    }
}

/// Status of an AI-suggested item.
#[derive(
    Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Default, Serialize, DeriveActiveEnum,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "ai_suggestion_status"
)]
pub enum AiSuggestionStatus {
    /// Suggestion is pending user review
    #[sea_orm(string_value = "pending")]
    #[default]
    Pending,
    /// User accepted the suggestion (converted to real entity)
    #[sea_orm(string_value = "accepted")]
    Accepted,
    /// User dismissed the suggestion
    #[sea_orm(string_value = "dismissed")]
    Dismissed,
}

impl std::fmt::Display for AiSuggestionStatus {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiSuggestionStatus::Pending => write!(fmt, "pending"),
            AiSuggestionStatus::Accepted => write!(fmt, "accepted"),
            AiSuggestionStatus::Dismissed => write!(fmt, "dismissed"),
        }
    }
}
