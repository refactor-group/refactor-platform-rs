use super::error::{EntityApiErrorKind, Error};
use entity::agreements::{ActiveModel, Entity, Model};
use entity::Id;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};

use log::*;

pub async fn create(
    db: &DatabaseConnection,
    agreement_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    debug!("New Agreement Model to be inserted: {:?}", agreement_model);

    let now = chrono::Utc::now();

    let agreement_active_model: ActiveModel = ActiveModel {
        coaching_session_id: Set(agreement_model.coaching_session_id),
        body: Set(agreement_model.body),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(agreement_active_model.save(db).await?.try_into_model()?)
}

pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(agreement) => {
            debug!("Existing Agreement model to be Updated: {:?}", agreement);

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(agreement.id),
                coaching_session_id: Unchanged(agreement.coaching_session_id),
                body: Set(model.body),
                user_id: Unchanged(agreement.user_id),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(agreement.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            debug!("Agreement with id {} not found", id);

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
    use entity::{agreements::Model, Id};
    use sea_orm::{DatabaseBackend, MockDatabase};

    #[tokio::test]
    async fn create_returns_a_new_agreement_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let agreement_model = Model {
            id: Id::new_v4(),
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("This is a agreement".to_owned()),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![agreement_model.clone()]])
            .into_connection();

        let agreement = create(&db, agreement_model.clone().into(), Id::new_v4()).await?;

        assert_eq!(agreement.id, agreement_model.id);

        Ok(())
    }

    #[tokio::test]
    async fn update_returns_an_updated_agreement_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let agreement_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("This is a agreement".to_owned()),
            user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![agreement_model.clone()],
                vec![agreement_model.clone()],
            ])
            .into_connection();

        let agreement = update(&db, agreement_model.id, agreement_model.clone()).await?;

        assert_eq!(agreement.body, agreement_model.body);

        Ok(())
    }
}
