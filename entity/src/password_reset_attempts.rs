use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "password_reset_attempts"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    /// SHA-256 hex digest of the normalized email (lowercased, trimmed).
    /// Opaque key — no FK to `users`, because attempts are recorded for
    /// unknown emails too (uniform enumeration-safe handling).
    pub email_hash: String,
    #[serde(skip_deserializing)]
    pub attempted_at: DateTimeWithTimeZone,
}

impl ActiveModelBehavior for ActiveModel {}
