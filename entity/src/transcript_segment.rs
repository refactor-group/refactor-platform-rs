use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::transcript_segment::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "transcript_segments")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    pub transcription_id: Id,
    pub speaker_label: String,
    pub text: String,
    pub start_ms: i32,
    pub end_ms: i32,
    pub confidence: Option<f64>,
    /// Sentiment label as returned by AssemblyAI: "positive", "neutral", or "negative".
    pub sentiment: Option<String>,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Transcriptions",
        from = "transcription_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    pub transcription: BelongsTo<super::transcription::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
