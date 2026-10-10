use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::transcript_suggestion_target::Target;
use crate::Id;

/// Where an extraction run is in its lifecycle.
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
    enum_name = "transcript_analysis_status"
)]
#[schema(as = entity::transcript_analysis::Status)]
pub enum Status {
    #[sea_orm(string_value = "pending")]
    #[default]
    Pending,
    #[sea_orm(string_value = "processing")]
    Processing,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "failed")]
    Failed,
}

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::transcript_analysis::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "transcript_analyses")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    pub transcription_id: Id,
    pub target: Target,
    pub status: Status,
    pub attempts: i16,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub claimed_at: Option<DateTimeWithTimeZone>,
    pub error_message: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTimeWithTimeZone,
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
