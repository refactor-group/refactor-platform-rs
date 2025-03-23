use super::error::{EntityApiErrorKind, Error};
use crate::{organization::Entity, uuid_parse_str};
use chrono::Utc;
use entity::{
    coachees, coaches, coaching_relationships, organizations::*, prelude::Organizations, Id,
};
use sea_orm::{
    entity::prelude::*, sea_query, ActiveValue::Set, ActiveValue::Unchanged, DatabaseConnection,
    JoinType, QuerySelect, TryIntoModel,
};
use slugify::slugify;
use std::collections::HashMap;

use log::*;

pub async fn create(db: &DatabaseConnection, organization_model: Model) -> Result<Model, Error> {
    debug!(
        "New Organization Model to be inserted: {:?}",
        organization_model
    );

    let now = Utc::now();
    let name = organization_model.name;

    let organization_active_model: ActiveModel = ActiveModel {
        logo: Set(organization_model.logo),
        name: Set(name.clone()),
        slug: Set(slugify!(name.as_str())),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(organization_active_model.insert(db).await?)
}

pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let organization = find_by_id(db, id).await?;

    let active_model: ActiveModel = ActiveModel {
        id: Unchanged(organization.id),
        logo: Set(model.logo),
        name: Set(model.name),
        slug: Unchanged(organization.slug),
        updated_at: Unchanged(organization.updated_at),
        created_at: Unchanged(organization.created_at),
    };
    Ok(active_model.update(db).await?.try_into_model()?)
}

pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let organization_model = find_by_id(db, id).await?;
    organization_model.delete(db).await?;
    Ok(())
}

pub async fn find_all(db: &DatabaseConnection) -> Result<Vec<Model>, Error> {
    Ok(Entity::find().all(db).await?)
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn find_with_coaches_coachees(
    db: &DatabaseConnection,
    id: Id,
) -> Result<(Model, Vec<coaches::Model>, Vec<coachees::Model>), Error> {
    let organization = find_by_id(db, id).await?;
    let coaches = organization.find_related(coaches::Entity).all(db).await?;
    let coachees = organization.find_related(coachees::Entity).all(db).await?;
    Ok((organization, coaches, coachees))
}

pub async fn find_by(
    db: &DatabaseConnection,
    params: HashMap<String, String>,
) -> Result<Vec<Model>, Error> {
    let mut query = Entity::find();

    for (key, value) in params {
        match key.as_str() {
            "user_id" => {
                let user_uuid = uuid_parse_str(&value)?;
                query = by_user(query, user_uuid).await;
            }
            _ => {
                return Err(Error {
                    source: None,
                    error_kind: EntityApiErrorKind::InvalidQueryTerm,
                });
            }
        }
    }

    Ok(query.distinct().all(db).await?)
}

pub async fn find_by_user(db: &DatabaseConnection, user_id: Id) -> Result<Vec<Model>, Error> {
    let organizations = by_user(Entity::find(), user_id).await.all(db).await?;

    Ok(organizations)
}

async fn by_user(query: Select<Organizations>, user_id: Id) -> Select<Organizations> {
    query
        .join(JoinType::InnerJoin, Relation::CoachingRelationships.def())
        .filter(
            sea_query::Condition::any()
                .add(coaching_relationships::Column::CoachId.eq(user_id))
                .add(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .distinct()
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::{organizations, Id};
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn find_all_returns_a_list_of_records_when_present() -> Result<(), Error> {
        let now = Utc::now();
        let organizations = vec![vec![
            organizations::Model {
                id: Id::new_v4(),
                name: "Organization One".to_owned(),
                slug: "organization-one".to_owned(),
                created_at: now.into(),
                updated_at: now.into(),
                logo: None,
            },
            organizations::Model {
                id: Id::new_v4(),
                name: "Organization One".to_owned(),
                slug: "organization-one".to_owned(),
                created_at: now.into(),
                updated_at: now.into(),
                logo: None,
            },
        ]];
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(organizations.clone())
            .into_connection();

        assert_eq!(find_all(&db).await?, organizations[0]);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_user_returns_all_records_associated_with_user() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = find_by_user(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT DISTINCT "organizations"."id", "organizations"."name", "organizations"."logo", "organizations"."slug", "organizations"."created_at", "organizations"."updated_at" FROM "refactor_platform"."organizations" INNER JOIN "refactor_platform"."coaching_relationships" ON "organizations"."id" = "coaching_relationships"."organization_id" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2"#,
                [user_id.clone().into(), user_id.into()]
            )]
        );

        Ok(())
    }
}
