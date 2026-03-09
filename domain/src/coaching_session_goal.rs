use crate::error::Error;
use entity_api::coaching_session_goal as CoachingSessionGoalApi;
use entity_api::coaching_sessions_goals::Model;
use entity_api::Id;
use sea_orm::DatabaseConnection;

pub async fn create(
    db: &DatabaseConnection,
    coaching_session_id: Id,
    goal_id: Id,
) -> Result<Model, Error> {
    Ok(CoachingSessionGoalApi::create(db, coaching_session_id, goal_id).await?)
}

pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    Ok(CoachingSessionGoalApi::delete_by_id(db, id).await?)
}

pub async fn find_by_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(CoachingSessionGoalApi::find_by_session_id(db, coaching_session_id).await?)
}

pub async fn find_by_goal_id(db: &DatabaseConnection, goal_id: Id) -> Result<Vec<Model>, Error> {
    Ok(CoachingSessionGoalApi::find_by_goal_id(db, goal_id).await?)
}
