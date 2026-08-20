//! `SeaORM` Entity for the coaching_session_reminders table.
//! Per-(user, coaching_session) reminder claim; at most one row per pair.

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::coaching_session_reminders::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_reminders"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
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
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::UserId",
        to = "super::users::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Users,
    #[sea_orm(
        belongs_to = "super::coaching_sessions::Entity",
        from = "Column::CoachingSessionId",
        to = "super::coaching_sessions::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    CoachingSessions,
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl Related<super::coaching_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingSessions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
