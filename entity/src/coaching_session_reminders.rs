//! `SeaORM` Entity for the coaching_session_reminders table.
//! Per-(user, coaching_session) reminder claim; at most one row per pair.

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::coaching_session_reminders::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_reminders"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    pub user_id: Id,
    pub coaching_session_id: Id,
    /// The `date` value this reminder was sent for, naive UTC like `date` itself.
    /// Holding the start rather than a "sent at" timestamp is what makes a reschedule
    /// re-arm the reminder: the sweep claims pairs whose stored value `IS DISTINCT
    /// FROM` the session's current start, so moving a session makes it due again with
    /// no other code path having to clear the claim.
    pub sent_for_start: DateTime,
    /// Regenerated on every claim. Identifies which claim a caller holds, so a delivery
    /// that outlasts a reclaim cannot confirm or release the newer one.
    pub claim_id: Id,
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
