use crate::actions::Model;
use crate::error::Error;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{actions, actions_user, query};
use sea_orm::DatabaseConnection;

pub use entity_api::action::{
    create, create_with_assignees, delete_by_id, find_by_id, find_by_id_with_assignees,
    find_by_user, update, update_status, update_with_assignees, ActionWithAssignees,
    AssigneeFilter, FindByUserParams, Scope,
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

    // Batch fetch all assignees for all actions in one query (avoids N+1 issue)
    let action_ids = actions.iter().map(|a| a.id).collect();
    let mut assignees_map = actions_user::find_assignees_for_actions(db, action_ids).await?;

    // Build results with assignees from the map
    let mut result = Vec::with_capacity(actions.len());
    for action in actions {
        let assignee_ids = assignees_map.remove(&action.id).unwrap_or_default();
        result.push(ActionWithAssignees {
            action,
            assignee_ids,
        });
    }

    Ok(result)
}
