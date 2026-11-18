//! `SeaORM` Entity for the coaching_session_pending_invites table.
//! One row per invite owed to one participant: a session's own invite or a series'.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Id;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::coaching_session_pending_invites::Model)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "coaching_session_pending_invites"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    /// The participant the invite is owed to.
    pub user_id: Id,
    /// The session whose own invite is owed; `None` for a series'.
    pub coaching_session_id: Option<Id>,
    /// The series whose invite is owed; `None` for a session's.
    pub coaching_session_series_id: Option<Id>,
    /// Failed sends so far.
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
    pub coaching_session: BelongsTo<Option<super::coaching_sessions::Entity>>,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "CoachingSessionSeries",
        from = "coaching_session_series_id",
        to = "id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    pub coaching_session_series: BelongsTo<Option<super::coaching_session_series::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
