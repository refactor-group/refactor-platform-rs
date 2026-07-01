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
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "pipeline_provider")]
#[schema(as = domain::pipeline_provider::Provider)]
pub enum Provider {
    #[sea_orm(string_value = "recall_ai")]
    #[default]
    RecallAi,

    #[sea_orm(string_value = "llm_gateway")]
    LlmGateway,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RecallAi => write!(f, "RecallAi"),
            Self::LlmGateway => write!(f, "LlmGateway"),
        }
    }
}
