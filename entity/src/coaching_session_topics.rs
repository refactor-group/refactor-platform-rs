//! `SeaORM` Entity.

use crate::topic_priority::Priority;
use crate::topic_status::Status;
use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::coaching_session_topics::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_topics"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub coaching_session_id: Id,
    pub body: String,
    #[serde(skip_deserializing)]
    pub user_id: Id,
    // Backend-internal ordering index. Never crosses the wire in either direction.
    #[serde(skip)]
    pub display_order: i32,
    // Coachee-set priority; null when unset.
    pub priority: Option<Priority>,
    // Lifecycle status; NOT NULL, defaults to Open.
    pub status: Status,
    // Provenance for carried-over topics; set server-side, null normally.
    #[serde(skip_deserializing)]
    pub carried_from_topic_id: Option<Id>,
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
        belongs_to = "super::users::Entity",
        from = "Column::UserId",
        to = "super::users::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Users,
}

impl Related<super::coaching_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingSessions.def()
    }
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
