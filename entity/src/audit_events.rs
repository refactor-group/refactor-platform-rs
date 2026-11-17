//! `SeaORM` Entity for the audit_events table.
//! An append-only, per-organization hash chain of who changed what.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use crate::Id;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(schema_name = "refactor_platform", table_name = "audit_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Id,
    pub organization_id: Id,
    /// Null when the system made the change.
    pub actor_user_id: Option<Id>,
    /// An `Action`, stored as its `as_str()` text.
    #[sea_orm(column_type = "Text")]
    pub action: String,
    /// What kind of record `subject_id` names.
    #[sea_orm(column_type = "Text")]
    pub subject_type: String,
    pub subject_id: Id,
    /// Small facts about the change, ids only.
    #[sea_orm(column_type = "JsonBinary")]
    pub details: serde_json::Value,
    pub occurred_at: DateTimeWithTimeZone,
    /// The hash of the organization's previous event; null on its first.
    #[sea_orm(column_type = "Text", nullable)]
    pub prev_hash: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub hash: String,
}

// No relations: events outlive the organization, users and records they describe.
impl ActiveModelBehavior for ActiveModel {}

/// What an audit event records. Stored as snake_case text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    RelationshipCreated,
    RelationshipDeleted,
    MemberAdded,
    MemberRemoved,
    MemberRoleChanged,
    RelationshipKindChanged,
}

impl Action {
    /// The action's stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Action::RelationshipCreated => "relationship_created",
            Action::RelationshipDeleted => "relationship_deleted",
            Action::MemberAdded => "member_added",
            Action::MemberRemoved => "member_removed",
            Action::MemberRoleChanged => "member_role_changed",
            Action::RelationshipKindChanged => "relationship_kind_changed",
        }
    }

    /// The kind of record the action changes, stored as the event's `subject_type`.
    pub fn subject_type(self) -> &'static str {
        match self {
            Action::RelationshipCreated
            | Action::RelationshipDeleted
            | Action::RelationshipKindChanged => "coaching_relationship",
            Action::MemberAdded | Action::MemberRemoved | Action::MemberRoleChanged => "user",
        }
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::Iterable;

    use super::*;

    #[test]
    fn as_str_matches_the_serde_value_of_every_action() {
        for action in Action::iter() {
            assert_eq!(
                serde_json::to_value(action).expect("serializes"),
                action.as_str(),
                "{action:?}"
            );
        }
    }
}
