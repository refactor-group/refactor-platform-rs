//! `SeaORM` Entity for actions_users junction table.
//! Represents the many-to-many relationship between actions and users (assignees).

use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::actions_users::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "actions_users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[serde(skip_deserializing)]
    pub id: Id,
    pub action_id: Id,
    pub user_id: Id,
    #[serde(skip_deserializing)]
    pub created_at: DateTimeWithTimeZone,
    #[serde(skip_deserializing)]
    pub updated_at: DateTimeWithTimeZone,
    #[serde(skip)]
    #[sea_orm(
        belongs_to,
        relation_enum = "Actions",
        from = "action_id",
        to = "id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    pub action: BelongsTo<super::actions::Entity>,
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
}

impl ActiveModelBehavior for ActiveModel {}
