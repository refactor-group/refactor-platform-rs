use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::transcript_suggestion_target::Target;
use crate::Id;

/// Review state of a proposed item.
#[derive(
    Debug,
    Clone,
    Copy,
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
    enum_name = "transcript_suggestion_status"
)]
#[schema(as = entity::transcript_suggestion::Status)]
pub enum Status {
    #[sea_orm(string_value = "pending")]
    #[default]
    Pending,
    #[sea_orm(string_value = "accepted")]
    Accepted,
    #[sea_orm(string_value = "dismissed")]
    Dismissed,
}

/// One item the model proposed from a transcript: the proposal, its source quote and its review
/// state.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::transcript_suggestion::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "transcript_suggestions"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    pub coaching_session_id: Id,
    pub transcript_analysis_id: Id,
    pub target: Target,
    #[schema(value_type = Object)]
    pub proposal: Json,
    pub segment_id: Id,
    pub quote: String,
    pub duplicate_of_id: Option<Id>,
    pub status: Status,
    pub resolved_by: Option<Id>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub resolved_at: Option<DateTimeWithTimeZone>,
    pub created_entity_id: Option<Id>,
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
        relation_enum = "TranscriptAnalyses",
        from = "transcript_analysis_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    pub transcript_analysis: BelongsTo<super::transcript_analysis::Entity>,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "TranscriptSegments",
        from = "segment_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    pub segment: BelongsTo<super::transcript_segment::Entity>,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Users",
        from = "resolved_by",
        to = "id",
        on_update = "NoAction",
        on_delete = "SetNull"
    )]
    pub resolver: BelongsTo<Option<super::users::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
