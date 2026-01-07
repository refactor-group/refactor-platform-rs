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

    /// User who stated this item (from speaker diarization)
    pub stated_by_user_id: Option<Id>,

    /// User who should complete this item (from LeMUR analysis)
    /// NULL for agreements since they are mutual commitments with no single assignee
    pub assigned_to_user_id: Option<Id>,

    /// Link to the transcript segment for provenance tracking
    pub source_segment_id: Option<Id>,

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

    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::StatedByUserId",
        to = "super::users::Column::Id",
        on_update = "NoAction",
        on_delete = "SetNull"
    )]
    StatedByUser,

    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::AssignedToUserId",
        to = "super::users::Column::Id",
        on_update = "NoAction",
        on_delete = "SetNull"
    )]
    AssignedToUser,

    #[sea_orm(
        belongs_to = "super::transcript_segments::Entity",
        from = "Column::SourceSegmentId",
        to = "super::transcript_segments::Column::Id",
        on_update = "NoAction",
        on_delete = "SetNull"
    )]
    SourceSegment,
}

impl Related<super::transcriptions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Transcriptions.def()
    }
}

impl Related<super::transcript_segments::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SourceSegment.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
