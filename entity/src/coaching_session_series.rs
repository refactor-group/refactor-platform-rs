use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = domain::coaching_session_series::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_series"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub coaching_relationship_id: Id,
    #[sea_orm(column_type = "JsonBinary")]
    #[schema(value_type = Object)]
    pub rule: serde_json::Value,
    #[serde(skip_deserializing)]
    pub ical_sequence: i32,
    pub created_by_user_id: Id,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: DateTimeWithTimeZone,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "CoachingRelationships",
        from = "coaching_relationship_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    pub coaching_relationship: BelongsTo<super::coaching_relationships::Entity>,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Users",
        from = "created_by_user_id",
        to = "id",
        on_update = "NoAction",
        on_delete = "Restrict"
    )]
    pub user: BelongsTo<super::users::Entity>,
    #[serde(skip)]
    #[sea_orm(has_many, relation_enum = "CoachingSessions")]
    pub coaching_sessions: HasMany<super::coaching_sessions::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
