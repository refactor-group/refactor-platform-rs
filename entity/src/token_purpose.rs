use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "token_purpose")]
pub enum TokenPurpose {
    #[sea_orm(string_value = "setup")]
    Setup,
    #[sea_orm(string_value = "password_reset")]
    PasswordReset,
}

impl std::fmt::Display for TokenPurpose {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenPurpose::Setup => write!(fmt, "setup"),
            TokenPurpose::PasswordReset => write!(fmt, "password_reset"),
        }
    }
}
