//! `SeaORM` Entity. Generated by sea-orm-codegen 0.12.3

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = domain::coaching_sessions::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "coaching_sessions")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub coaching_relationship_id: Id,
    #[serde(skip_deserializing)]
    pub collab_document_name: Option<String>,
    pub date: DateTime,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)] // Applies to OpenAPI schema
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)] // Applies to OpenAPI schema
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::actions::Entity")]
    Actions,
    #[sea_orm(has_many = "super::agreements::Entity")]
    Agreements,
    #[sea_orm(
        belongs_to = "super::coaching_relationships::Entity",
        from = "Column::CoachingRelationshipId",
        to = "super::coaching_relationships::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    CoachingRelationships,
    #[sea_orm(has_many = "super::notes::Entity")]
    Notes,
    #[sea_orm(has_many = "super::overarching_goals::Entity")]
    OverarchingGoals,
}

impl Related<super::actions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Actions.def()
    }
}

impl Related<super::agreements::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Agreements.def()
    }
}

impl Related<super::coaching_relationships::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingRelationships.def()
    }
}

impl Related<super::notes::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Notes.def()
    }
}

impl Related<super::overarching_goals::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OverarchingGoals.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
