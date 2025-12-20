use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Sentiment analysis result for a transcript segment.
#[derive(Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "sentiment")]
pub enum Sentiment {
    #[sea_orm(string_value = "positive")]
    Positive,
    #[sea_orm(string_value = "neutral")]
    Neutral,
    #[sea_orm(string_value = "negative")]
    Negative,
}

impl std::fmt::Display for Sentiment {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sentiment::Positive => write!(fmt, "positive"),
            Sentiment::Neutral => write!(fmt, "neutral"),
            Sentiment::Negative => write!(fmt, "negative"),
        }
    }
}
