//! `SeaORM` Entity. Generated by sea-orm-codegen 0.12.3

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::coaching_relationships::Model)] // OpenAPI schema
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_relationships"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    #[serde(skip_deserializing)]
    #[sea_orm(unique)]
    pub organization_id: Id,
    pub coach_id: Id,
    pub coachee_id: Id,
    #[serde(skip_deserializing)]
    // TODO we need to make sure this is unique in the scope of an organization_id.
    // I did some research and there are two ways to do this:
    // 1. Create a unique constraint at the database level.
    // 2. Add application logic (probably in entity_api) to make the check.
    // We'll need to add a migration for that eventually.
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
    #[sea_orm(
        belongs_to = "super::organizations::Entity",
        from = "Column::OrganizationId",
        to = "super::organizations::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    Organizations,
    #[sea_orm(
        belongs_to = "super::coaches::Entity",
        from = "Column::CoachId",
        to = "super::coaches::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    Coaches,
    #[sea_orm(
        belongs_to = "super::coachees::Entity",
        from = "Column::CoacheeId",
        to = "super::coachees::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    Coachees,
}

impl Related<super::organizations::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Organizations.def()
    }
}

impl Related<super::coaches::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Coaches.def()
    }
}

impl Related<super::coachees::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Coachees.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

// Alias to be used with queries
#[derive(DeriveIden, Clone, Copy)]
pub struct RelationshipAsCoach;

// Alias to be used with queries
#[derive(DeriveIden, Clone, Copy)]
pub struct RelationshipAsCoachee;
