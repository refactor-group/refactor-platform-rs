use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(
    Debug,
    Clone,
    Eq,
    PartialEq,
    EnumIter,
    Deserialize,
    Serialize,
    DeriveActiveEnum,
    Default,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
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

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = domain::transcription::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "transcriptions")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    pub coaching_session_id: Id,
    pub meeting_recording_id: Id,
    /// Recall.ai's transcript ID, returned from the "Create Async Transcript" API.
    pub external_id: String,
    /// Recall.ai's recording UUID — required for transcript API calls (`/recording/{id}/...`).
    /// Internal server-side correlation ID; not exposed to API clients.
    #[serde(skip_serializing)]
    pub recall_recording_id: Option<String>,
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
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "CoachingSessions",
        from = "coaching_session_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    pub coaching_session: BelongsTo<super::coaching_sessions::Entity>,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "MeetingRecordings",
        from = "meeting_recording_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    pub meeting_recording: BelongsTo<super::meeting_recording::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
