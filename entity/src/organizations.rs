//! `SeaORM` Entity. Generated by sea-orm-codegen 0.12.3

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, ToSchema, Serialize, Deserialize)]
#[schema(as = entity::organizations::Model)] // OpenAPI schema
#[sea_orm(schema_name = "refactor_platform", table_name = "organizations")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    #[sea_orm(unique)]
    pub name: String,
    pub logo: Option<String>,
    #[serde(skip_deserializing)]
    #[sea_orm(unique)]
    pub slug: String,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)] // Applies to OpenAPI schema
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)] // Applies to OpenAPI schema
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::coaching_relationships::Entity")]
    CoachingRelationships,

    #[sea_orm(has_many = "super::organizations_users::Entity")]
    OrganizationsUsers,
}

impl Related<super::coaching_relationships::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingRelationships.def()
    }
}

impl Related<super::coaches::Entity> for Entity {
    fn to() -> RelationDef {
        super::coaching_relationships::Relation::Coaches.def()
    }

    fn via() -> Option<RelationDef> {
        Some(
            super::coaching_relationships::Relation::Organizations
                .def()
                .rev(),
        )
    }
}

impl Related<super::coachees::Entity> for Entity {
    fn to() -> RelationDef {
        super::coaching_relationships::Relation::Coachees.def()
    }

    fn via() -> Option<RelationDef> {
        Some(
            super::coaching_relationships::Relation::Organizations
                .def()
                .rev(),
        )
    }
}

// Through relationship for users by way of organizations_users
// organizations -> organizations_users -> users
impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        super::organizations_users::Relation::Users.def()
    }

    fn via() -> Option<RelationDef> {
        Some(
            super::organizations_users::Relation::Organizations
                .def()
                .rev(),
        )
    }
}

impl ActiveModelBehavior for ActiveModel {}
