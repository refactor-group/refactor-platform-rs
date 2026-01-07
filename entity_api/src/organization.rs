use super::error::{EntityApiErrorKind, Error};
use crate::{organization::Entity, uuid_parse_str};
use chrono::Utc;
use entity::{organizations::*, prelude::Organizations, roles, user_roles, Id};
use sea_orm::{
    entity::prelude::*, ActiveValue::Set, ActiveValue::Unchanged, ConnectionTrait, JoinType,
    QuerySelect, TryIntoModel,
};
use slugify::slugify;
use std::collections::HashMap;

use log::*;

pub async fn create(db: &impl ConnectionTrait, organization_model: Model) -> Result<Model, Error> {
    debug!("New Organization Model to be inserted: {organization_model:?}");

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

pub async fn update(db: &impl ConnectionTrait, id: Id, model: Model) -> Result<Model, Error> {
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

pub async fn delete_by_id(db: &impl ConnectionTrait, id: Id) -> Result<(), Error> {
    let organization_model = find_by_id(db, id).await?;
    organization_model.delete(db).await?;
    Ok(())
}

pub async fn find_all(db: &impl ConnectionTrait) -> Result<Vec<Model>, Error> {
    Ok(Entity::find().all(db).await?)
}

pub async fn find_by_id(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn find_by(
    db: &impl ConnectionTrait,
    params: HashMap<String, String>,
) -> Result<Vec<Model>, Error> {
    if let Some((key, value)) = params.into_iter().next() {
        match key.as_str() {
            "user_id" => {
                let user_uuid = uuid_parse_str(&value)?;
                return find_by_user(db, user_uuid).await;
            }
            _ => {
                return Err(Error {
                    source: None,
                    error_kind: EntityApiErrorKind::InvalidQueryTerm,
                });
            }
        }
    }

    // If no params provided, return all organizations
    Ok(Entity::find().all(db).await?)
}

pub async fn find_by_user(db: &impl ConnectionTrait, user_id: Id) -> Result<Vec<Model>, Error> {
    // Check if user is a super admin (has role = 'super_admin' with organization_id = NULL)
    let is_super_admin = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::Role.eq(roles::Role::SuperAdmin))
        .filter(user_roles::Column::OrganizationId.is_null())
        .one(db)
        .await?
        .is_some();

    let organizations = if is_super_admin {
        // Super admins have access to all organizations
        let orgs = Entity::find().all(db).await?;
        orgs
    } else {
        // Regular users only see organizations they're explicitly assigned to
        let orgs = by_user(Entity::find(), user_id).await.all(db).await?;
        orgs
    };

    Ok(organizations)
}

async fn by_user(query: Select<Organizations>, user_id: Id) -> Select<Organizations> {
    query
        .join(JoinType::InnerJoin, Relation::UserRoles.def())
        .filter(user_roles::Column::UserId.eq(user_id))
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
        // Mock empty results for both the super admin check and the organizations query
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<user_roles::Model, Vec<user_roles::Model>, _>(vec![vec![]])
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let user_id = Id::new_v4();
        let _ = find_by_user(&db, user_id).await;

        // Should make two queries: first check if user is super admin, then fetch organizations
        assert_eq!(
            db.into_transaction_log(),
            [
                Transaction::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    r#"SELECT "user_roles"."id", CAST("user_roles"."role" AS "text"), "user_roles"."organization_id", "user_roles"."user_id", "user_roles"."created_at", "user_roles"."updated_at" FROM "refactor_platform"."user_roles" WHERE "user_roles"."user_id" = $1 AND "user_roles"."role" = (CAST($2 AS "role")) AND "user_roles"."organization_id" IS NULL LIMIT $3"#,
                    [user_id.into(), "super_admin".into(), 1u64.into()]
                ),
                Transaction::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    r#"SELECT DISTINCT "organizations"."id", "organizations"."name", "organizations"."logo", "organizations"."slug", "organizations"."created_at", "organizations"."updated_at" FROM "refactor_platform"."organizations" INNER JOIN "refactor_platform"."user_roles" ON "organizations"."id" = "user_roles"."organization_id" WHERE "user_roles"."user_id" = $1"#,
                    [user_id.into()]
                )
            ]
        );

        Ok(())
    }
}
