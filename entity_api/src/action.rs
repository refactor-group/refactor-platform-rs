use super::error::{EntityApiErrorKind, Error};
use entity::actions::{ActiveModel, Entity, Model};
use entity::{status::Status, Id};
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};

use log::*;

pub async fn create(
    db: &DatabaseConnection,
    action_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    debug!("New Action Model to be inserted: {action_model:?}");

    let now = chrono::Utc::now();

    let action_active_model: ActiveModel = ActiveModel {
        coaching_session_id: Set(action_model.coaching_session_id),
        user_id: Set(user_id),
        body: Set(action_model.body),
        status: Set(action_model.status),
        due_by: Set(action_model.due_by),
        status_changed_at: Set(now.into()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(action_active_model.save(db).await?.try_into_model()?)
}

pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(action) => {
            debug!("Existing Action model to be Updated: {action:?}");

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(action.id),
                coaching_session_id: Unchanged(action.coaching_session_id),
                user_id: Unchanged(model.user_id),
                body: Set(model.body),
                due_by: Set(model.due_by),
                status: Set(model.status),
                status_changed_at: Set(chrono::Utc::now().into()),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(action.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Action with id {id} not found");

            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

pub async fn update_status(
    db: &DatabaseConnection,
    id: Id,
    status: Status,
) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(action) => {
            debug!("Existing Action model to be Updated: {action:?}");

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(action.id),
                coaching_session_id: Unchanged(action.coaching_session_id),
                user_id: Unchanged(action.user_id),
                body: Unchanged(action.body),
                due_by: Unchanged(action.due_by),
                status: Set(status),
                status_changed_at: Set(chrono::Utc::now().into()),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(action.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Action with id {id} not found");

            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let result = find_by_id(db, id).await?;

    result.delete(db).await?;

    Ok(())
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::{actions::Model, Id};
    use sea_orm::{DatabaseBackend, MockDatabase};

    #[tokio::test]
    async fn create_returns_a_new_action_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let action_model = Model {
            id: Id::new_v4(),
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("This is a action".to_owned()),
            due_by: Some(now.into()),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action_model.clone()]])
            .into_connection();

        let action = create(&db, action_model.clone().into(), Id::new_v4()).await?;

        assert_eq!(action.id, action_model.id);

        Ok(())
    }

    #[tokio::test]
    async fn update_returns_an_updated_action_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let action_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            due_by: Some(now.into()),
            body: Some("This is a action".to_owned()),
            user_id: Id::new_v4(),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action_model.clone()], vec![action_model.clone()]])
            .into_connection();

        let action = update(&db, action_model.id, action_model.clone()).await?;

        assert_eq!(action.body, action_model.body);

        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_an_updated_action_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let action_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            due_by: Some(now.into()),
            body: Some("This is a action".to_owned()),
            user_id: Id::new_v4(),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let updated_action_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            due_by: Some(now.into()),
            body: Some("This is a action".to_owned()),
            user_id: Id::new_v4(),
            status_changed_at: now.into(),
            status: Status::Completed,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![action_model.clone()],
                vec![updated_action_model.clone()],
            ])
            .into_connection();

        let action = update_status(&db, action_model.id, Status::Completed).await?;

        assert_eq!(action.status, Status::Completed);

        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_error_when_action_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let result = update_status(&db, Id::new_v4(), Status::Completed).await;

        assert_eq!(result.is_err(), true);

        Ok(())
    }
}
