//! SeaORM Entity for transcriptions table.
//! Stores transcription data from AssemblyAI.

use crate::transcription_status::TranscriptionStatus;
use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::transcriptions::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "transcriptions")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,

    pub meeting_recording_id: Id,

    /// AssemblyAI transcript ID
    pub assemblyai_transcript_id: Option<String>,

    /// Current status of the transcription
    pub status: TranscriptionStatus,

    /// Full transcription text
    #[sea_orm(column_type = "Text")]
    pub full_text: Option<String>,

    /// AI-generated summary of the transcription
    #[sea_orm(column_type = "Text")]
    pub summary: Option<String>,

    /// Confidence score from AssemblyAI (0.0 - 1.0)
    pub confidence_score: Option<f64>,

    /// Total word count
    pub word_count: Option<i32>,

    /// Language code (default: en)
    pub language_code: Option<String>,

    /// Error message if transcription failed
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
        belongs_to = "super::meeting_recordings::Entity",
        from = "Column::MeetingRecordingId",
        to = "super::meeting_recordings::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    MeetingRecordings,

    #[sea_orm(has_many = "super::transcript_segments::Entity")]
    TranscriptSegments,

    #[sea_orm(has_many = "super::ai_suggested_items::Entity")]
    AiSuggestedItems,
}

impl Related<super::meeting_recordings::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MeetingRecordings.def()
    }
}

impl Related<super::transcript_segments::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::TranscriptSegments.def()
    }
}

impl Related<super::ai_suggested_items::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::AiSuggestedItems.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
