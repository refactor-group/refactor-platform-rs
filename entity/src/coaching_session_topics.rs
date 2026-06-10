//! `SeaORM` Entity.

use crate::topic_priority::Priority;
use crate::topic_status::Status;
use crate::Id;
use sea_orm::entity::prelude::*;
use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// Server-only undo buffer: the row's pre-defer state, captured at defer time so undefer
// can restore it faithfully. Coupled to the defer operation, not the topic schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct TopicDeferSnapshot {
    pub coaching_session_id: Id,
    pub status: crate::topic_status::Status,
    pub display_order: i32,
    pub moved_from_session_id: Option<Id>,
    pub updated_at: DateTimeWithTimeZone,
}

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
    // Provenance for a moved topic: the session it was last moved out of. Server-set; null normally.
    #[serde(skip_deserializing)]
    pub moved_from_session_id: Option<Id>,
    // Server-only undo buffer for a faithful undefer; never crosses the wire.
    #[sea_orm(column_type = "JsonBinary", nullable)]
    #[serde(skip)]
    pub pre_defer_snapshot: Option<TopicDeferSnapshot>,
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
