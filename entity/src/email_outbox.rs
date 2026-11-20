//! `SeaORM` Entity for the email_outbox table.
//! One row per email owed to one recipient, sent by the outbox sweep.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use crate::Id;

/// Where a row is in its life: waiting to send, or finished one way or the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum State {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "sent")]
    Sent,
    #[sea_orm(string_value = "failed")]
    Failed,
}

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(schema_name = "refactor_platform", table_name = "email_outbox")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    /// Stable notification name, for logs and audit.
    pub kind: String,
    /// The conversation this email is ordered within.
    pub subject_key: String,
    pub recipient_user_id: Id,
    /// Order within the batch that wrote it.
    pub position: i16,
    /// The provider request body exactly as sent; cleared once the row finishes.
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub request: Option<Json>,
    pub state: State,
    /// Attempts so far.
    pub attempts: i16,
    /// Earliest next attempt, naive UTC.
    pub not_before: DateTime,
    /// The sweep's lease: which claim holds the row.
    pub claim_id: Option<Id>,
    /// The sweep's lease: when it lapses, naive UTC.
    pub lease_until: Option<DateTime>,
    /// The last failure as the provider described it.
    pub last_error: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    /// When the row became sent or failed.
    pub finished_at: Option<DateTimeWithTimeZone>,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Users",
        from = "recipient_user_id",
        to = "id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    pub recipient: BelongsTo<super::users::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
