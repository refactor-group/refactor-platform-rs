use crate::actions::Model;
use crate::error::Error;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{action_assignee, actions, query};
use sea_orm::DatabaseConnection;

pub use entity_api::action::{
    create, create_with_assignees, delete_by_id, find_by_id, find_by_id_with_assignees,
    find_by_user, update, update_status, update_with_assignees, ActionWithAssignees,
    AssigneeFilter, Scope, UserActionsQuery,
};

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<actions::Column>,
{
    let actions = query::find_by::<actions::Entity, actions::Column, P>(db, params).await?;
    Ok(actions)
}

/// Finds actions with their assignee IDs.
///
/// This fetches actions matching the given parameters and includes
/// the assignee user IDs for each action.
pub async fn find_by_with_assignees<P>(
    db: &DatabaseConnection,
    params: P,
) -> Result<Vec<ActionWithAssignees>, Error>
where
    P: IntoQueryFilterMap + QuerySort<actions::Column>,
{
    let actions = query::find_by::<actions::Entity, actions::Column, P>(db, params).await?;

    let mut result = Vec::with_capacity(actions.len());
    for action in actions {
        let assignee_ids = action_assignee::find_user_ids_by_action_id(db, action.id).await?;
        result.push(ActionWithAssignees {
            action,
            assignee_ids,
        });
    }

    Ok(result)
}
