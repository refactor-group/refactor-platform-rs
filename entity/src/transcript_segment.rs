use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[sea_orm(schema_name = "refactor_platform", table_name = "transcript_segments")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
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
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::transcription::Entity",
        from = "Column::TranscriptionId",
        to = "super::transcription::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Transcriptions,
}

impl Related<super::transcription::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Transcriptions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
