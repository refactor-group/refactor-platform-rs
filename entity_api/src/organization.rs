use super::error::{EntityApiErrorCode, Error};
use crate::organization::Entity;
use chrono::Utc;
use entity::{coaching_relationships, organizations::*, Id};
use sea_orm::{
    entity::prelude::*, sea_query, ActiveValue, ActiveValue::Set, ActiveValue::Unchanged,
    DatabaseConnection, JoinType, QuerySelect, TryIntoModel,
};
use serde_json::json;

use log::*;

pub async fn create(db: &DatabaseConnection, organization_model: Model) -> Result<Model, Error> {
    let organization_active_model: ActiveModel = ActiveModel {
        name: Set(organization_model.name.to_owned()),
        ..Default::default()
    };
    debug!(
        "New Organization ActiveModel to be inserted: {:?}",
        organization_active_model
    );

    Ok(organization_active_model.insert(db).await?)
}

pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = find_by_id(db, id).await?;

    match result {
        Some(organization) => {
            debug!(
                "Existing Organization model to be Updated: {:?}",
                organization
            );

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(organization.id),
                external_id: Set(Uuid::new_v4()),
                logo: Set(model.logo),
                name: Set(model.name),
                updated_at: Unchanged(organization.updated_at),
                created_at: Unchanged(organization.created_at),
            };
            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => Err(Error {
            inner: None,
            error_code: EntityApiErrorCode::RecordNotFound,
        }),
    }
}

pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let result = find_by_id(db, id).await?;

    match result {
        Some(organization_model) => {
            debug!(
                "Existing Organization model to be deleted: {:?}",
                organization_model
            );

            organization_model.delete(db).await?;
            Ok(())
        }
        None => Err(Error {
            inner: None,
            error_code: EntityApiErrorCode::RecordNotFound,
        }),
    }
}

pub async fn find_all(db: &DatabaseConnection) -> Result<Vec<Model>, Error> {
    Ok(Entity::find().all(db).await?)
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Option<Model>, Error> {
    let organization = Entity::find_by_id(id).one(db).await?;
    debug!("Organization found: {:?}", organization);

    Ok(organization)
}

pub async fn find_by_user(db: &DatabaseConnection, user_id: Id) -> Result<Vec<Model>, Error> {
    let organizations = Entity::find()
        .join(JoinType::InnerJoin, Relation::CoachingRelationships.def())
        .filter(
            sea_query::Condition::any()
                .add(coaching_relationships::Column::CoachId.eq(user_id))
                .add(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .all(db)
        .await?;

    Ok(organizations)
}

pub(crate) async fn seed_database(db: &DatabaseConnection) {
    let organization_names = [
        "Jim Hodapp Coaching",
        "Caleb Coaching",
        "Enterprise Software",
    ];
    let uuid = Uuid::new_v4();

    let now = Utc::now();

    for name in organization_names {
        let organization = entity::organizations::ActiveModel::from_json(json!({
            "name": name,
            "external_id": uuid,
            "created_at": now,
            "updated_at": now,
        }))
        .unwrap();

        assert_eq!(
            organization,
            entity::organizations::ActiveModel {
                id: ActiveValue::NotSet,
                name: ActiveValue::Set(Some(name.to_owned())),
                external_id: ActiveValue::Set(uuid),
                logo: ActiveValue::NotSet,
                created_at: ActiveValue::Set(now.into()),
                updated_at: ActiveValue::Set(now.into()),
            }
        );

        match Entity::insert(organization)
            .on_conflict(
                // on conflict do update
                sea_query::OnConflict::column(Column::ExternalId)
                    .update_column(Column::Name)
                    .to_owned(),
            )
            .exec(db)
            .await
        {
            Ok(_) => info!("Succeeded in seeding new organization entity."),
            Err(e) => {
                error!("Failed to insert or update organization entity when seeding user data: {e}")
            }
        };
    }
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::organizations;
    use sea_orm::{prelude::Uuid, DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn find_all_returns_a_list_of_records_when_present() -> Result<(), Error> {
        let now = Utc::now();
        let organizations = vec![vec![
            organizations::Model {
                id: 1,
                name: Some("Organization One".to_owned()),
                created_at: now.into(),
                updated_at: now.into(),
                logo: None,
                external_id: Uuid::new_v4(),
            },
            organizations::Model {
                id: 2,
                name: Some("Organization One".to_owned()),
                created_at: now.into(),
                updated_at: now.into(),
                logo: None,
                external_id: Uuid::new_v4(),
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

        let user_id = 1;
        let _ = find_by_user(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "organizations"."id", "organizations"."external_id", "organizations"."name", "organizations"."logo", "organizations"."created_at", "organizations"."updated_at" FROM "refactor_platform"."organizations" INNER JOIN "refactor_platform"."coaching_relationships" ON "organizations"."id" = "coaching_relationships"."organization_id" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2"#,
                [user_id.into(), user_id.into()]
            )]
        );

        Ok(())
    }
}
