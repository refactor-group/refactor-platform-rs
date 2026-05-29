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
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "cost_unit")]
#[schema(as = domain::cost_unit::Unit)]
pub enum Unit {
    #[sea_orm(string_value = "minutes")]
    #[default]
    Minutes,

    #[sea_orm(string_value = "hours")]
    Hours,

    #[sea_orm(string_value = "tokens")]
    Tokens,
}
