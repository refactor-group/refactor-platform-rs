//! `SeaORM` Entity.

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Metadata for one image pasted into a coaching session's notes. Every column is
/// server-derived — the client uploads bytes and nothing else — so the whole model
/// skips deserialization.
#[sea_orm::compact_model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::coaching_session_images::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_images"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    #[serde(skip_deserializing)]
    pub coaching_session_id: Id,
    #[serde(skip_deserializing)]
    pub uploaded_by_id: Id,
    // Internal object storage path. Never crosses the wire: it would expose the bucket layout.
    #[serde(skip)]
    pub storage_key: String,
    #[serde(skip_deserializing)]
    pub mime_type: String,
    #[serde(skip_deserializing)]
    pub byte_size: i64,
    // Null when the format's headers did not yield dimensions.
    #[serde(skip_deserializing)]
    pub width: Option<i32>,
    #[serde(skip_deserializing)]
    pub height: Option<i32>,
    // Internal bookkeeping for the deferred purge, never a client's business.
    #[serde(skip)]
    pub deleted_at: Option<DateTimeWithTimeZone>,
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
        from = "Column::UploadedById",
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
