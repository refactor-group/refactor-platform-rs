use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "status")]
pub enum Status {
    #[sea_orm(string_value = "not_started")]
    NotStarted,
    #[sea_orm(string_value = "in_progress")]
    InProgress,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "wont_do")]
    WontDo,
}

impl std::default::Default for Status {
    fn default() -> Self {
        Self::InProgress
    }
}

impl From<&str> for Status {
    fn from(value: &str) -> Self {
        match value {
            "not_started" => Self::NotStarted,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "wont_do" => Self::WontDo,
            _ => Self::InProgress,
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => write!(f, "not_started"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::WontDo => write!(f, "wont_do"),
        }
    }
}
