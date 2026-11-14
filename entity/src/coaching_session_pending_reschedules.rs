//! `SeaORM` Entity for the coaching_session_pending_reschedules table.
//! One row per session whose schedule changed since participants were last told.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Id;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::coaching_session_pending_reschedules::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_pending_reschedules"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub coaching_session_id: Id,
    /// The start participants were last told, naive UTC.
    pub announced_date: DateTime,
    /// The duration participants were last told.
    pub announced_duration_minutes: i16,
    /// When the first unannounced change was recorded.
    pub unannounced_since: DateTime,
    /// When the latest change was recorded.
    pub changed_at: DateTime,
    /// Bumped by every recorded change.
    pub revision: i32,
    /// Failed sends since the last delivered one.
    pub attempts: i16,
    /// The sweep's lease: which claim holds the row.
    pub claim_id: Option<Id>,
    /// The sweep's lease: when the row was claimed.
    pub claimed_at: Option<DateTime>,
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
}

impl ActiveModelBehavior for ActiveModel {}
