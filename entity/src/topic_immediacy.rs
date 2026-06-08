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
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "topic_immediacy")]
#[schema(as = entity::topic_immediacy::Immediacy)]
pub enum Immediacy {
    #[sea_orm(string_value = "neutral")]
    #[default]
    Neutral,
    #[sea_orm(string_value = "can_wait")]
    CanWait,
    #[sea_orm(string_value = "soon")]
    Soon,
    #[sea_orm(string_value = "pressing")]
    Pressing,
}
