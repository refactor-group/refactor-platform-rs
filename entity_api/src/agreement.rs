use super::error::{EntityApiErrorKind, Error};
use crate::uuid_parse_str;
use entity::agreements::{self, ActiveModel, Entity, Model};
use entity::Id;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};
use std::collections::HashMap;

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

    match result {
        Some(agreement_model) => {
            debug!(
                "Existing Agreement model to be deleted: {:?}",
                agreement_model
            );

            agreement_model.delete(db).await?;
            Ok(())
        }
        None => Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        }),
    }
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Option<Model>, Error> {
    match Entity::find_by_id(id).one(db).await {
        Ok(Some(agreement)) => {
            debug!("Agreement found: {:?}", agreement);

            Ok(Some(agreement))
        }
        Ok(None) => {
            error!("Agreement with id {} not found", id);

            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
        Err(err) => {
            error!("Error finding Agreement with id {}: {:?}", id, err);

            Err(Error {
                source: Some(err),
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
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

                query = query.filter(agreements::Column::CoachingSessionId.eq(coaching_session_id));
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
    use entity::{agreements::Model, Id};
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

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

    #[tokio::test]
    async fn find_by_id_returns_agreement_associated_with_id() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let agreement_id = Id::new_v4();

        let _ = find_by_id(&db, agreement_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "agreements"."id", "agreements"."coaching_session_id", "agreements"."body", "agreements"."user_id", "agreements"."created_at", "agreements"."updated_at" FROM "refactor_platform"."agreements" WHERE "agreements"."id" = $1 LIMIT $2"#,
                [agreement_id.into(), sea_orm::Value::BigUnsigned(Some(1))]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_returns_all_agreements_associated_with_coaching_session() -> Result<(), Error>
    {
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
                r#"SELECT "agreements"."id", "agreements"."coaching_session_id", "agreements"."body", "agreements"."user_id", "agreements"."created_at", "agreements"."updated_at" FROM "refactor_platform"."agreements" WHERE "agreements"."coaching_session_id" = $1"#,
                [coaching_session_id.into()]
            )]
        );

        Ok(())
    }
}
