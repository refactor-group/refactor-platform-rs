use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Default, Serialize, DeriveActiveEnum,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "role")]
pub enum Role {
    #[sea_orm(string_value = "user")]
    #[default]
    User,
    #[sea_orm(string_value = "admin")]
    Admin,
}

impl std::fmt::Display for Role {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => write!(fmt, "user"),
            Role::Admin => write!(fmt, "admin"),
        }
    }
}
