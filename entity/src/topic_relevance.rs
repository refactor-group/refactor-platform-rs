use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(
    Debug,
    Clone,
    Eq,
    PartialEq,
    EnumIter,
    Deserialize,
    Serialize,
    DeriveActiveEnum,
    Default,
    ToSchema,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "topic_relevance")]
#[schema(as = entity::topic_relevance::Relevance)]
pub enum Relevance {
    #[sea_orm(string_value = "neutral")]
    #[default]
    Neutral,
    #[sea_orm(string_value = "peripheral")]
    Peripheral,
    #[sea_orm(string_value = "worth_exploring")]
    WorthExploring,
    #[sea_orm(string_value = "central")]
    Central,
}
