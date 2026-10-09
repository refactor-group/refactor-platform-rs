use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Id;

/// How a transcript participant was attributed to a platform user.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, EnumIter, Serialize, Deserialize, DeriveActiveEnum, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "speaker_match_source"
)]
pub enum MatchSource {
    #[sea_orm(string_value = "account")]
    Account,
    #[sea_orm(string_value = "elimination")]
    Elimination,
}

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::transcript_participant::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "transcript_participants"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    pub transcription_id: Id,
    pub provider_participant_id: String,
    // Raw provider data, internal only.
    #[serde(skip_serializing)]
    pub display_name: Option<String>,
    pub is_host: Option<bool>,
    pub platform: Option<String>,
    // Raw provider data, internal only.
    #[serde(skip_serializing)]
    pub platform_account_id: Option<String>,
    // Raw provider data, internal only.
    #[serde(skip_serializing)]
    pub extra_data: Option<Json>,
    pub user_id: Option<Id>,
    pub match_source: Option<MatchSource>,
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
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Users",
        from = "user_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "SetNull"
    )]
    pub user: BelongsTo<Option<super::users::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
