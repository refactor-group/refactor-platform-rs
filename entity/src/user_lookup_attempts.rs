use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(schema_name = "refactor_platform", table_name = "user_lookup_attempts")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    /// The user who performed the lookup. No FK to `users`: rows are
    /// rate-limiter state, not a relation.
    pub requester_user_id: Id,
    #[serde(skip_deserializing)]
    pub attempted_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
