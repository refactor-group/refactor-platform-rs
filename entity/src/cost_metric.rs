use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    EnumIter,
    Deserialize,
    Serialize,
    DeriveActiveEnum,
    Default,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "cost_metric")]
#[schema(as = domain::cost_metric::Metric)]
pub enum Metric {
    #[sea_orm(string_value = "bot_minutes")]
    #[default]
    BotMinutes,

    #[sea_orm(string_value = "transcription_hours")]
    TranscriptionHours,

    #[sea_orm(string_value = "llm_tokens")]
    LlmTokens,
}
