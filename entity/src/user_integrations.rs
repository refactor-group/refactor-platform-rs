//! SeaORM Entity for user_integrations table.
//! Stores encrypted API credentials for external service integrations.

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::user_integrations::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "user_integrations")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,

    pub user_id: Id,

    // Google OAuth (encrypted in database)
    pub google_access_token: Option<String>,
    pub google_refresh_token: Option<String>,
    pub google_token_expiry: Option<DateTimeWithTimeZone>,
    pub google_email: Option<String>,

    // Recall.ai (encrypted in database)
    pub recall_ai_api_key: Option<String>,
    pub recall_ai_region: Option<String>,
    pub recall_ai_verified_at: Option<DateTimeWithTimeZone>,

    // AssemblyAI (encrypted in database)
    pub assembly_ai_api_key: Option<String>,
    pub assembly_ai_verified_at: Option<DateTimeWithTimeZone>,

    /// Auto-approve AI suggestions without manual review
    /// When true, LeMUR-extracted actions/agreements are created directly
    /// When false (default), they become AI suggestions for coach review
    #[serde(default)]
    pub auto_approve_ai_suggestions: bool,

    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,

    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::UserId",
        to = "super::users::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Users,
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
