use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// Lifecycle status. NOT NULL with default Open (untriaged).
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
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "topic_status")]
#[schema(as = entity::topic_status::Status)]
pub enum Status {
    #[sea_orm(string_value = "open")]
    #[default]
    Open,
    #[sea_orm(string_value = "discussed")]
    Discussed,
    #[sea_orm(string_value = "deferred")]
    Deferred,
}
