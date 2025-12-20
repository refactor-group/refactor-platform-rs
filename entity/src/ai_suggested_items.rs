//! SeaORM Entity for ai_suggested_items table.
//! Stores AI-detected action items and agreements before user approval.

use crate::ai_suggestion::{AiSuggestionStatus, AiSuggestionType};
use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::ai_suggested_items::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "ai_suggested_items")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,

    pub transcription_id: Id,

    /// Type of suggestion (action or agreement)
    pub item_type: AiSuggestionType,

    /// The suggested content/text
    #[sea_orm(column_type = "Text")]
    pub content: String,

    /// Original transcript text this was extracted from
    #[sea_orm(column_type = "Text")]
    pub source_text: Option<String>,

    /// Confidence score from AI (0.0 - 1.0)
    pub confidence: Option<f64>,

    /// Current status of the suggestion
    pub status: AiSuggestionStatus,

    /// ID of the created Action or Agreement entity after acceptance
    pub accepted_entity_id: Option<Id>,

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
        belongs_to = "super::transcriptions::Entity",
        from = "Column::TranscriptionId",
        to = "super::transcriptions::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Transcriptions,
}

impl Related<super::transcriptions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Transcriptions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
