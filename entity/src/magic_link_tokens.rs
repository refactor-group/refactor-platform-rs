pub use crate::token_purpose::TokenPurpose;
use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(schema_name = "refactor_platform", table_name = "magic_link_tokens")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub user_id: Id,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub expires_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    pub created_at: DateTimeWithTimeZone,
    pub purpose: TokenPurpose,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Users",
        from = "user_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    pub user: BelongsTo<super::users::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
