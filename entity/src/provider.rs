use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum, Default,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "provider")]
pub enum Provider {
    #[sea_orm(string_value = "google")]
    #[default]
    Google,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Google => write!(f, "Google"),
        }
    }
}
