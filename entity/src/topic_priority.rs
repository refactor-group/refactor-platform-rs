use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// Coachee-set priority. Nullable at the column level, so no Default.
#[derive(
    Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum, ToSchema,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "topic_priority")]
#[schema(as = entity::topic_priority::Priority)]
pub enum Priority {
    #[sea_orm(string_value = "low")]
    Low,
    #[sea_orm(string_value = "medium")]
    Medium,
    #[sea_orm(string_value = "high")]
    High,
}
