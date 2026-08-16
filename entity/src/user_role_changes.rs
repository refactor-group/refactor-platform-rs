use super::roles::Role;
use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(schema_name = "refactor_platform", table_name = "user_role_changes")]
pub struct Model {
    #[sea_orm(primary_key)]
    #[serde(skip_deserializing)]
    pub id: Id,
    /// Null only for a future system-initiated change with no human actor.
    pub actor_user_id: Option<Uuid>,
    pub target_user_id: Uuid,
    /// Null for a global SuperAdmin grant, which belongs to no organization.
    pub organization_id: Option<Uuid>,
    /// Null on a first grant.
    pub previous_role: Option<Role>,
    /// Null on a removal.
    pub new_role: Option<Role>,
    #[serde(skip_deserializing)]
    pub changed_at: DateTimeWithTimeZone,
}

// No relations: the table carries no foreign keys so audit rows outlive their subjects.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
