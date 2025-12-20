//! SeaORM Entity for transcript_segments table.
//! Stores individual utterances with speaker diarization.

use crate::sentiment::Sentiment;
use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::transcript_segments::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "transcript_segments")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,

    pub transcription_id: Id,

    /// Speaker label from diarization (e.g., "Speaker A", "Speaker B")
    pub speaker_label: String,

    /// Mapped user ID if speaker has been identified
    pub speaker_user_id: Option<Id>,

    /// The spoken text for this segment
    #[sea_orm(column_type = "Text")]
    pub text: String,

    /// Start time in milliseconds from beginning of recording
    pub start_time_ms: i64,

    /// End time in milliseconds from beginning of recording
    pub end_time_ms: i64,

    /// Confidence score for this segment (0.0 - 1.0)
    pub confidence: Option<f64>,

    /// Sentiment analysis result for this segment
    pub sentiment: Option<Sentiment>,

    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::transcriptions::Entity",
        from = "Column::TranscriptionId",
        to = "super::transcriptions::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Transcriptions,

    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::SpeakerUserId",
        to = "super::users::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    Users,
}

impl Related<super::transcriptions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Transcriptions.def()
    }
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
