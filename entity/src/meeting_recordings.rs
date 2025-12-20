//! SeaORM Entity for meeting_recordings table.
//! Tracks meeting recordings from Recall.ai.

use crate::meeting_recording_status::MeetingRecordingStatus;
use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::meeting_recordings::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "meeting_recordings")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,

    pub coaching_session_id: Id,

    /// Recall.ai bot ID for this recording
    pub recall_bot_id: Option<String>,

    /// Current status of the recording
    pub status: MeetingRecordingStatus,

    /// URL to the recording (after processing)
    pub recording_url: Option<String>,

    /// Duration of the recording in seconds
    pub duration_seconds: Option<i32>,

    /// When the recording started
    #[schema(value_type = Option<String>, format = DateTime)]
    pub started_at: Option<DateTimeWithTimeZone>,

    /// When the recording ended
    #[schema(value_type = Option<String>, format = DateTime)]
    pub ended_at: Option<DateTimeWithTimeZone>,

    /// Error message if recording failed
    pub error_message: Option<String>,

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
        belongs_to = "super::coaching_sessions::Entity",
        from = "Column::CoachingSessionId",
        to = "super::coaching_sessions::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    CoachingSessions,

    #[sea_orm(has_one = "super::transcriptions::Entity")]
    Transcriptions,
}

impl Related<super::coaching_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingSessions.def()
    }
}

impl Related<super::transcriptions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Transcriptions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
