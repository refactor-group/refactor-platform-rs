use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, ToSchema, Serialize, Deserialize)]
#[schema(as = entity::users::Model)] // OpenAPI schema
#[sea_orm(schema_name = "refactor_platform", table_name = "users")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    #[sea_orm(unique)]
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub display_name: Option<String>,
    #[serde(skip)]
    pub password: String,
    pub github_username: Option<String>,
    pub github_profile_url: Option<String>,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)] // Applies to OpenAPI schema
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)] // Applies to OpenAPI schema
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        has_many = "super::coaching_relationships::Entity",
        from = "Column::Id",
        to = "super::coaching_relationships::Column::CoachId",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    CoachingRelationships,
}

impl ActiveModelBehavior for ActiveModel {}
