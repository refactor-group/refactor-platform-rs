use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum, Default,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "pat_status")]
pub enum PATStatus {
    #[sea_orm(string_value = "active")]
    #[default]
    Active,
    #[sea_orm(string_value = "inactive")]
    Inactive,
}

impl std::fmt::Display for PATStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Inactive => write!(f, "inactive"),
        }
    }
}
