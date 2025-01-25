use super::error::{EntityApiErrorKind, Error};
use crate::uuid_parse_str;
use entity::notes::{self, ActiveModel, Entity, Model};
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
    note_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    debug!("New Note Model to be inserted: {:?}", note_model);

    let now = chrono::Utc::now();

    let note_active_model: ActiveModel = ActiveModel {
        coaching_session_id: Set(note_model.coaching_session_id),
        body: Set(note_model.body),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(note_active_model.save(db).await?.try_into_model()?)
}

pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(note) => {
            debug!("Existing Note model to be Updated: {:?}", note);

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(note.id),
                coaching_session_id: Unchanged(note.coaching_session_id),
                body: Set(model.body),
                user_id: Unchanged(note.user_id),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(note.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Note with id {} not found", id);

            Err(Error {
                inner: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Option<Model>, Error> {
    match Entity::find_by_id(id).one(db).await {
        Ok(Some(note)) => {
            debug!("Note found: {:?}", note);

            Ok(Some(note))
        }
        Ok(None) => {
            error!("Note with id {} not found", id);

            Err(Error {
                inner: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
        Err(err) => {
            error!("Note with id {} not found and returned error {}", id, err);
            Err(Error {
                inner: None,
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

                query = query.filter(notes::Column::CoachingSessionId.eq(coaching_session_id));
            }
            _ => {
                return Err(Error {
                    inner: None,
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
    use entity::{notes::Model, Id};
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn create_returns_a_new_note_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let note_model = Model {
            id: Id::new_v4(),
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("This is a note".to_owned()),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![note_model.clone()]])
            .into_connection();

        let note = create(&db, note_model.clone().into(), Id::new_v4()).await?;

        assert_eq!(note.id, note_model.id);

        Ok(())
    }

    #[tokio::test]
    async fn update_returns_an_updated_note_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let note_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("This is a note".to_owned()),
            user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![note_model.clone()], vec![note_model.clone()]])
            .into_connection();

        let note = update(&db, note_model.id, note_model.clone()).await?;

        assert_eq!(note.body, note_model.body);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_returns_all_notes_associated_with_coaching_session() -> Result<(), Error> {
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
                r#"SELECT "notes"."id", "notes"."coaching_session_id", "notes"."body", "notes"."user_id", "notes"."created_at", "notes"."updated_at" FROM "refactor_platform"."notes" WHERE "notes"."coaching_session_id" = $1"#,
                [coaching_session_id.into()]
            )]
        );

        Ok(())
    }
}
