//! `SeaORM` Entity for coaching_sessions_goals junction table.
//! Represents the many-to-many relationship between coaching sessions and goals.

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_sessions_goals"
)]
pub struct Model {
    #[sea_orm(primary_key)]
    #[serde(skip_deserializing)]
    pub id: Id,
    pub coaching_session_id: Id,
    pub goal_id: Id,
    #[serde(skip_deserializing)]
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::coaching_sessions::Entity",
        from = "Column::CoachingSessionId",
        to = "super::coaching_sessions::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    CoachingSessions,
    #[sea_orm(
        belongs_to = "super::goals::Entity",
        from = "Column::GoalId",
        to = "super::goals::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Goals,
}

impl Related<super::coaching_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingSessions.def()
    }
}

impl Related<super::goals::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Goals.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
