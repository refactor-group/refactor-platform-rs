//! `SeaORM` Entity. Generated by sea-orm-codegen 0.12.3

pub use crate::roles::Role;
use crate::Id;
use axum_login::AuthUser;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// TODO: We should find a way to centralize the users/coaches/coachees types
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, ToSchema, Serialize, Deserialize)]
#[schema(as = domain::users::Model)] // OpenAPI schema
#[sea_orm(schema_name = "refactor_platform", table_name = "users")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    #[sea_orm(unique)]
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub display_name: Option<String>,
    #[serde(skip_serializing)]
    pub password: String,
    pub github_username: Option<String>,
    pub github_profile_url: Option<String>,
    #[sea_orm(default = "UTC")]
    pub timezone: String,
    #[sea_orm(default = "user")]
    #[serde(skip_deserializing)]
    pub role: Role,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)] // Applies to OpenAPI schema
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)] // Applies to OpenAPI schema
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::organizations_users::Entity")]
    OrganizationsUsers,
}

impl Related<super::organizations::Entity> for Entity {
    fn to() -> RelationDef {
        super::organizations_users::Relation::Organizations.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::organizations_users::Relation::Users.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl AuthUser for Model {
    type Id = crate::Id;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        self.password.as_bytes()
    }
}
