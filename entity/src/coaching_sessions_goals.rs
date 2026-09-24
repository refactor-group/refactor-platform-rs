//! `SeaORM` Entity for coaching_sessions_goals junction table.
//! Represents the many-to-many relationship between coaching sessions and goals.

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_sessions_goals"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[serde(skip_deserializing)]
    pub id: Id,
    pub coaching_session_id: Id,
    pub goal_id: Id,
    #[serde(skip_deserializing)]
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    pub updated_at: DateTimeWithTimeZone,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "CoachingSessions",
        from = "coaching_session_id",
        to = "id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    pub coaching_session: BelongsTo<super::coaching_sessions::Entity>,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Goals",
        from = "goal_id",
        to = "id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    pub goal: BelongsTo<super::goals::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
