use super::error::{EntityApiErrorKind, Error};
use entity::{
    coaching_relationships,
    coaching_sessions::{ActiveModel, Entity, Model},
    Id,
};
use log::debug;
use sea_orm::{entity::prelude::*, DatabaseConnection, Set, TryIntoModel};

pub async fn create(
    db: &DatabaseConnection,
    coaching_session_model: Model,
) -> Result<Model, Error> {
    debug!("New Coaching Session Model to be inserted: {coaching_session_model:?}");

    let now = chrono::Utc::now();

    let coaching_session_active_model: ActiveModel = ActiveModel {
        coaching_relationship_id: Set(coaching_session_model.coaching_relationship_id),
        date: Set(coaching_session_model.date),
        collab_document_name: Set(coaching_session_model.collab_document_name),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(coaching_session_active_model
        .save(db)
        .await?
        .try_into_model()?)
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn find_by_id_with_coaching_relationship(
    db: &DatabaseConnection,
    id: Id,
) -> Result<(Model, coaching_relationships::Model), Error> {
    if let Some(results) = Entity::find_by_id(id)
        .find_also_related(coaching_relationships::Entity)
        .one(db)
        .await?
    {
        if let Some(coaching_relationship) = results.1 {
            return Ok((results.0, coaching_relationship));
        }
    }
    Err(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn delete(db: &impl ConnectionTrait, coaching_session_id: Id) -> Result<(), Error> {
    Entity::delete_by_id(coaching_session_id).exec(db).await?;
    Ok(())
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn create_returns_a_new_coaching_session_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let coaching_session_model = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            date: chrono::Local::now().naive_utc(),
            collab_document_name: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![coaching_session_model.clone()]])
            .into_connection();

        let coaching_session = create(&db, coaching_session_model.clone().into()).await?;

        assert_eq!(coaching_session.id, coaching_session_model.id);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_returns_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let coaching_session_id = Id::new_v4();
        let _ = find_by_id(&db, coaching_session_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."created_at", "coaching_sessions"."updated_at" FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."id" = $1 LIMIT $2"#,
                [
                    coaching_session_id.into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_with_coaching_relationship_returns_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let coaching_session_id = Id::new_v4();
        let _ = find_by_id_with_coaching_relationship(&db, coaching_session_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id" AS "A_id", "coaching_sessions"."coaching_relationship_id" AS "A_coaching_relationship_id", "coaching_sessions"."collab_document_name" AS "A_collab_document_name", "coaching_sessions"."date" AS "A_date", "coaching_sessions"."created_at" AS "A_created_at", "coaching_sessions"."updated_at" AS "A_updated_at", "coaching_relationships"."id" AS "B_id", "coaching_relationships"."organization_id" AS "B_organization_id", "coaching_relationships"."coach_id" AS "B_coach_id", "coaching_relationships"."coachee_id" AS "B_coachee_id", "coaching_relationships"."slug" AS "B_slug", "coaching_relationships"."created_at" AS "B_created_at", "coaching_relationships"."updated_at" AS "B_updated_at" FROM "refactor_platform"."coaching_sessions" LEFT JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_sessions"."id" = $1 LIMIT $2"#,
                [
                    coaching_session_id.into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn delete_deletes_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let coaching_session_id = Id::new_v4();
        let _ = delete(&db, coaching_session_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"DELETE FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."id" = $1"#,
                [coaching_session_id.into(),]
            )]
        );

        Ok(())
    }
}
