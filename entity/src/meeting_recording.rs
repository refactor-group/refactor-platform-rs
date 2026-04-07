use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(
    Debug, Clone, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum, Default,
    ToSchema,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "meeting_recording_status"
)]
pub enum MeetingRecordingStatus {
    #[sea_orm(string_value = "pending")]
    #[default]
    Pending,
    #[sea_orm(string_value = "joining")]
    Joining,
    #[sea_orm(string_value = "waiting_room")]
    WaitingRoom,
    #[sea_orm(string_value = "in_meeting")]
    InMeeting,
    #[sea_orm(string_value = "recording")]
    Recording,
    #[sea_orm(string_value = "processing")]
    Processing,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "failed")]
    Failed,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[sea_orm(schema_name = "refactor_platform", table_name = "meeting_recordings")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub coaching_session_id: Id,
    pub bot_id: String,
    pub status: MeetingRecordingStatus,
    pub video_url: Option<String>,
    /// Internal only — pre-signed audio download URL from Recall.ai. Never sent to clients.
    #[serde(skip_serializing)]
    pub audio_url: Option<String>,
    pub duration_seconds: Option<i32>,
    pub started_at: Option<DateTimeWithTimeZone>,
    pub ended_at: Option<DateTimeWithTimeZone>,
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
}

impl Related<super::coaching_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingSessions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
