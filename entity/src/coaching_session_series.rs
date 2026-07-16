use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = domain::coaching_session_series::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_series"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub coaching_relationship_id: Id,
    #[sea_orm(column_type = "JsonBinary")]
    #[schema(value_type = Object)]
    pub rule: serde_json::Value,
    pub created_by_user_id: Id,
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
        belongs_to = "super::coaching_relationships::Entity",
        from = "Column::CoachingRelationshipId",
        to = "super::coaching_relationships::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    CoachingRelationships,
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::CreatedByUserId",
        to = "super::users::Column::Id",
        on_update = "NoAction",
        on_delete = "Restrict"
    )]
    Users,
    #[sea_orm(has_many = "super::coaching_sessions::Entity")]
    CoachingSessions,
}

impl Related<super::coaching_relationships::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingRelationships.def()
    }
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl Related<super::coaching_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingSessions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
