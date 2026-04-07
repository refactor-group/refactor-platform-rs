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
    enum_name = "transcription_status"
)]
pub enum TranscriptionStatus {
    #[sea_orm(string_value = "queued")]
    #[default]
    Queued,
    #[sea_orm(string_value = "processing")]
    Processing,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "failed")]
    Failed,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[sea_orm(schema_name = "refactor_platform", table_name = "transcriptions")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub coaching_session_id: Id,
    pub meeting_recording_id: Id,
    /// Recall.ai's transcript ID, returned from the "Create Async Transcript" API.
    pub external_id: String,
    pub status: TranscriptionStatus,
    pub language_code: Option<String>,
    pub speaker_count: Option<i16>,
    pub word_count: Option<i32>,
    pub duration_seconds: Option<i32>,
    pub confidence: Option<f64>,
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
    #[sea_orm(
        belongs_to = "super::meeting_recording::Entity",
        from = "Column::MeetingRecordingId",
        to = "super::meeting_recording::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    MeetingRecordings,
}

impl Related<super::coaching_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingSessions.def()
    }
}

impl Related<super::meeting_recording::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MeetingRecordings.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
