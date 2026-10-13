use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The entity type a transcript suggestion would become.
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
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "transcript_suggestion_target"
)]
#[schema(as = domain::transcript_suggestion_target::Target)]
pub enum Target {
    #[sea_orm(string_value = "action")]
    #[default]
    Action,
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Action => write!(f, "Action"),
        }
    }
}
