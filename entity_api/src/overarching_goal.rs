use super::error::{EntityApiErrorKind, Error};
use crate::uuid_parse_str;
use entity::overarching_goals::{self, ActiveModel, Entity, Model};
use entity::{status::Status, Id};
use sea_orm::ActiveValue;
use sea_orm::{
    entity::prelude::*,
    ActiveModelTrait,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};
use std::collections::HashMap;

use log::*;

pub async fn create(
    db: &DatabaseConnection,
    overarching_goal_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    debug!(
        "New Overarching Goal Model to be inserted: {:?}",
        overarching_goal_model
    );

    let now = chrono::Utc::now();

    let overarching_goal_active_model: ActiveModel = ActiveModel {
        coaching_session_id: Set(overarching_goal_model.coaching_session_id),
        user_id: Set(user_id),
        title: Set(overarching_goal_model.title),
        body: Set(overarching_goal_model.body),
        status: Set(overarching_goal_model.status),
        status_changed_at: Set(Some(now.into())),
        completed_at: Unchanged(overarching_goal_model.completed_at),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(overarching_goal_active_model
        .save(db)
        .await?
        .try_into_model()?)
}

pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(overarching_goal) => {
            debug!(
                "Existing Overarching Goal model to be Updated: {:?}",
                overarching_goal
            );

            // Automatically update status_changed_at if the last status and new status differ:
            let av_status_changed_at: ActiveValue<Option<DateTimeWithTimeZone>> =
                if model.status != overarching_goal.status {
                    debug!("Updating status_changed_at for Overarching Goal to now");
                    Set(Some(chrono::Utc::now().into()))
                } else {
                    Unchanged(model.status_changed_at)
                };

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(overarching_goal.id),
                coaching_session_id: Unchanged(overarching_goal.coaching_session_id),
                user_id: Unchanged(overarching_goal.user_id),
                body: Set(model.body),
                title: Set(model.title),
                status: Set(model.status),
                status_changed_at: av_status_changed_at,
                completed_at: Set(model.completed_at),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(overarching_goal.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Overarching Goal with id {} not found", id);

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
        Some(overarching_goal) => {
            debug!(
                "Existing Overarching Goal model to be Updated: {:?}",
                overarching_goal
            );

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(overarching_goal.id),
                coaching_session_id: Unchanged(overarching_goal.coaching_session_id),
                user_id: Unchanged(overarching_goal.user_id),
                body: Unchanged(overarching_goal.body),
                title: Unchanged(overarching_goal.title),
                status: Set(status),
                status_changed_at: Set(Some(chrono::Utc::now().into())),
                completed_at: Unchanged(overarching_goal.completed_at),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(overarching_goal.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Overarching Goal with id {} not found", id);

            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn find_by(
    db: &DatabaseConnection,
    query_params: HashMap<String, String>,
) -> Result<Vec<Model>, Error> {
    let mut query = Entity::find();

    for (key, value) in query_params {
        match key.as_str() {
            "coaching_session_id" => {
                let coaching_session_id = uuid_parse_str(&value)?;

                query = query
                    .filter(overarching_goals::Column::CoachingSessionId.eq(coaching_session_id));
            }
            _ => {
                return Err(Error {
                    source: None,
                    error_kind: EntityApiErrorKind::InvalidQueryTerm,
                });
            }
        }
    }

    Ok(query.all(db).await?)
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::{overarching_goals::Model, Id};
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn create_returns_a_new_overarching_goal_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let overarching_goal_model = Model {
            id: Id::new_v4(),
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            title: Some("title".to_owned()),
            body: Some("This is a overarching_goal".to_owned()),
            status_changed_at: None,
            status: Default::default(),
            completed_at: Some(now.into()),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![overarching_goal_model.clone()]])
            .into_connection();

        let overarching_goal =
            create(&db, overarching_goal_model.clone().into(), Id::new_v4()).await?;

        assert_eq!(overarching_goal.id, overarching_goal_model.id);

        Ok(())
    }

    #[tokio::test]
    async fn update_returns_an_updated_overarching_goal_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let overarching_goal_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            title: Some("title".to_owned()),
            body: Some("This is a overarching_goal".to_owned()),
            user_id: Id::new_v4(),
            completed_at: Some(now.into()),
            status_changed_at: None,
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![overarching_goal_model.clone()],
                vec![overarching_goal_model.clone()],
            ])
            .into_connection();

        let overarching_goal = update(
            &db,
            overarching_goal_model.id,
            overarching_goal_model.clone(),
        )
        .await?;

        assert_eq!(overarching_goal.body, overarching_goal_model.body);

        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_an_updated_overarching_goal_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let overarching_goal_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            title: Some("title".to_owned()),
            body: Some("This is a overarching_goal".to_owned()),
            user_id: Id::new_v4(),
            completed_at: Some(now.into()),
            status_changed_at: None,
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let updated_overarching_goal_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            title: Some("title".to_owned()),
            body: Some("This is a overarching_goal".to_owned()),
            user_id: Id::new_v4(),
            completed_at: Some(now.into()),
            status_changed_at: Some(now.into()),
            status: Status::Completed,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![overarching_goal_model.clone()],
                vec![updated_overarching_goal_model.clone()],
            ])
            .into_connection();

        let overarching_goal =
            update_status(&db, overarching_goal_model.id, Status::Completed).await?;

        assert_eq!(overarching_goal.status, Status::Completed);

        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_error_when_overarching_goal_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let result = update_status(&db, Id::new_v4(), Status::Completed).await;

        assert_eq!(result.is_err(), true);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_returns_all_overarching_goals_associated_with_coaching_session(
    ) -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut query_params = HashMap::new();
        let coaching_session_id = Id::new_v4();

        query_params.insert(
            "coaching_session_id".to_owned(),
            coaching_session_id.to_string(),
        );

        let _ = find_by(&db, query_params).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "overarching_goals"."id", "overarching_goals"."coaching_session_id", "overarching_goals"."user_id", "overarching_goals"."title", "overarching_goals"."body", CAST("overarching_goals"."status" AS text), "overarching_goals"."status_changed_at", "overarching_goals"."completed_at", "overarching_goals"."created_at", "overarching_goals"."updated_at" FROM "refactor_platform"."overarching_goals" WHERE "overarching_goals"."coaching_session_id" = $1"#,
                [coaching_session_id.into()]
            )]
        );

        Ok(())
    }
}
