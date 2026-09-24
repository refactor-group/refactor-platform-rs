//! `SeaORM` Entity for the coaching_session_views table.
//! Per-(user, coaching_session) "last viewed at" marker; at most one row per pair.

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::coaching_session_views::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_views"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub user_id: Id,
    pub coaching_session_id: Id,
    pub last_viewed_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    pub updated_at: DateTimeWithTimeZone,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Users",
        from = "user_id",
        to = "id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    pub user: BelongsTo<super::users::Entity>,
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
}

impl ActiveModelBehavior for ActiveModel {}
