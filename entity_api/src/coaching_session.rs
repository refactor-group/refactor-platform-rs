use super::error::{EntityApiErrorCode, Error};
use crate::{naive_date_parse_str, uuid_parse_str};
use entity::{
    coaching_relationships,
    coaching_sessions::{self, ActiveModel, Entity, Model},
    Id,
};
use log::debug;
use sea_orm::{entity::prelude::*, DatabaseConnection, Set, TryIntoModel};
use std::collections::HashMap;

pub async fn create(
    db: &DatabaseConnection,
    coaching_session_model: Model,
) -> Result<Model, Error> {
    debug!(
        "New Coaching Session Model to be inserted: {:?}",
        coaching_session_model
    );

    let now = chrono::Utc::now();

    let coaching_session_active_model: ActiveModel = ActiveModel {
        coaching_relationship_id: Set(coaching_session_model.coaching_relationship_id),
        date: Set(coaching_session_model.date),
        timezone: Set(coaching_session_model.timezone),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(coaching_session_active_model
        .save(db)
        .await?
        .try_into_model()?)
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Option<Model>, Error> {
    Ok(Entity::find_by_id(id).one(db).await?)
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
        inner: None,
        error_code: EntityApiErrorCode::RecordNotFound,
    })
}

pub async fn find_by(
    db: &DatabaseConnection,
    params: HashMap<String, String>,
) -> Result<Vec<Model>, Error> {
    let mut query = Entity::find();

    for (key, value) in params {
        match key.as_str() {
            "coaching_relationship_id" => {
                let coaching_relationship_id = uuid_parse_str(&value)?;
                query = query.filter(
                    coaching_sessions::Column::CoachingRelationshipId.eq(coaching_relationship_id),
                )
            }
            "from_date" => {
                let from_date = naive_date_parse_str(&value)?;
                query = query.filter(coaching_sessions::Column::Date.gt(from_date));
            }
            "to_date" => {
                let to_date = naive_date_parse_str(&value)?;
                query = query.filter(coaching_sessions::Column::Date.lt(to_date));
            }
            _ => {
                return Err(Error {
                    inner: None,
                    error_code: EntityApiErrorCode::InvalidQueryTerm,
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
    use chrono::NaiveDate;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn create_returns_a_new_coaching_session_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let coaching_session_model = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            date: chrono::Local::now().naive_utc(),
            timezone: "Americas/Chicago".to_owned(),
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
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."date", "coaching_sessions"."timezone", "coaching_sessions"."created_at", "coaching_sessions"."updated_at" FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."id" = $1 LIMIT $2"#,
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
                r#"SELECT "coaching_sessions"."id" AS "A_id", "coaching_sessions"."coaching_relationship_id" AS "A_coaching_relationship_id", "coaching_sessions"."date" AS "A_date", "coaching_sessions"."timezone" AS "A_timezone", "coaching_sessions"."created_at" AS "A_created_at", "coaching_sessions"."updated_at" AS "A_updated_at", "coaching_relationships"."id" AS "B_id", "coaching_relationships"."organization_id" AS "B_organization_id", "coaching_relationships"."coach_id" AS "B_coach_id", "coaching_relationships"."coachee_id" AS "B_coachee_id", "coaching_relationships"."created_at" AS "B_created_at", "coaching_relationships"."updated_at" AS "B_updated_at" FROM "refactor_platform"."coaching_sessions" LEFT JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_sessions"."id" = $1 LIMIT $2"#,
                [
                    coaching_session_id.into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_coaching_relationships_returns_all_records_associated_with_coaching_relationship(
    ) -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut query_params = HashMap::new();
        let coaching_relationship_id = Id::new_v4();

        query_params.insert(
            "coaching_relationship_id".to_owned(),
            coaching_relationship_id.to_string(),
        );

        let _ = find_by(&db, query_params).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."date", "coaching_sessions"."timezone", "coaching_sessions"."created_at", "coaching_sessions"."updated_at" FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."coaching_relationship_id" = $1"#,
                [coaching_relationship_id.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_from_date_returns_all_records_after_date() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut query_params = HashMap::new();
        let from_date = NaiveDate::from_ymd_opt(2021, 1, 1).unwrap();

        query_params.insert("from_date".to_owned(), from_date.to_string());

        let _ = find_by(&db, query_params).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."date", "coaching_sessions"."timezone", "coaching_sessions"."created_at", "coaching_sessions"."updated_at" FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."date" > $1"#,
                [from_date.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_to_date_returns_all_records_before_date() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut query_params = HashMap::new();
        let to_date = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();

        query_params.insert("to_date".to_owned(), to_date.to_string());

        let _ = find_by(&db, query_params).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."date", "coaching_sessions"."timezone", "coaching_sessions"."created_at", "coaching_sessions"."updated_at" FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."date" < $1"#,
                [to_date.into()]
            )]
        );

        Ok(())
    }
}
