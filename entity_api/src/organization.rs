use super::error::{EntityApiErrorKind, Error};
use crate::{organization::Entity, uuid_parse_str};
use chrono::Utc;
use entity::{
    coaching_relationships, coaching_sessions, organizations::*, prelude::Organizations, roles,
    user_roles, Id,
};
use sea_orm::{
    entity::prelude::*, ActiveValue::Set, ActiveValue::Unchanged, ConnectionTrait, JoinType,
    QuerySelect, TransactionTrait, TryIntoModel,
};
use slugify::slugify;
use std::collections::HashMap;

use log::*;

/// Archive-status selector applied to organization reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    Active, // archived_at IS NULL
    Archived, // archived_at IS NOT NULL
    All,      // no archive filter
}

fn apply_status_filter(
    query: Select<Organizations>,
    status: StatusFilter,
) -> Select<Organizations> {
    match status {
        StatusFilter::Active => query.filter(Column::ArchivedAt.is_null()),
        StatusFilter::Archived => query.filter(Column::ArchivedAt.is_not_null()),
        StatusFilter::All => query,
    }
}

pub async fn create(db: &impl TransactionTrait, organization_model: Model) -> Result<Model, Error> {
    debug!("New Organization Model to be inserted: {organization_model:?}");

    let txn = db.begin().await?;
    let now = Utc::now();
    let name = organization_model.name;

    // Name-collision pre-check.
    if Entity::find()
        .filter(Column::Name.eq(&name))
        .one(&txn)
        .await?
        .is_some()
    {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationNameTaken { name },
        });
    }

    let organization_active_model: ActiveModel = ActiveModel {
        logo: Set(organization_model.logo),
        name: Set(name.clone()),
        slug: Set(slugify!(name.as_str())),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    let inserted = match organization_active_model.insert(&txn).await {
        Ok(model) => model,
        Err(insert_err) => {
            // Race backstop: a concurrent insert may have claimed the name.
            if Entity::find()
                .filter(Column::Name.eq(&name))
                .one(&txn)
                .await?
                .is_some()
            {
                return Err(Error {
                    source: None,
                    error_kind: EntityApiErrorKind::OrganizationNameTaken { name },
                });
            }
            return Err(insert_err.into());
        }
    };

    txn.commit().await?;
    Ok(inserted)
}

pub async fn update(db: &impl TransactionTrait, id: Id, model: Model) -> Result<Model, Error> {
    let txn = db.begin().await?;
    let organization = find_by_id(&txn, id).await?;

    // Name-collision pre-check against other orgs.
    if Entity::find()
        .filter(Column::Name.eq(&model.name))
        .filter(Column::Id.ne(id))
        .one(&txn)
        .await?
        .is_some()
    {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationNameTaken { name: model.name },
        });
    }

    let active_model: ActiveModel = ActiveModel {
        id: Unchanged(organization.id),
        logo: Set(model.logo),
        name: Set(model.name),
        slug: Unchanged(organization.slug),
        updated_at: Unchanged(organization.updated_at),
        created_at: Unchanged(organization.created_at),
        archived_at: Unchanged(organization.archived_at),
        archived_by: Unchanged(organization.archived_by),
    };
    let updated = active_model.update(&txn).await?.try_into_model()?;
    txn.commit().await?;
    Ok(updated)
}

/// Archive an organization (idempotent). Re-archiving is a no-op that avoids
/// timestamp churn.
pub async fn archive(db: &impl TransactionTrait, id: Id, archived_by: Id) -> Result<Model, Error> {
    let txn = db.begin().await?;
    let organization = find_by_id(&txn, id).await?;

    if organization.archived_at.is_some() {
        txn.commit().await?;
        return Ok(organization);
    }

    let now = Utc::now();
    let active_model: ActiveModel = ActiveModel {
        id: Unchanged(organization.id),
        logo: Unchanged(organization.logo),
        name: Unchanged(organization.name),
        slug: Unchanged(organization.slug),
        created_at: Unchanged(organization.created_at),
        updated_at: Set(now.into()),
        archived_at: Set(Some(now.into())),
        archived_by: Set(Some(archived_by)),
    };
    let archived = active_model.update(&txn).await?.try_into_model()?;
    txn.commit().await?;
    Ok(archived)
}

/// Unarchive an organization (idempotent). Unarchiving an active org is a no-op.
pub async fn unarchive(db: &impl TransactionTrait, id: Id) -> Result<Model, Error> {
    let txn = db.begin().await?;
    let organization = find_by_id(&txn, id).await?;

    if organization.archived_at.is_none() {
        txn.commit().await?;
        return Ok(organization);
    }

    let now = Utc::now();
    let active_model: ActiveModel = ActiveModel {
        id: Unchanged(organization.id),
        logo: Unchanged(organization.logo),
        name: Unchanged(organization.name),
        slug: Unchanged(organization.slug),
        created_at: Unchanged(organization.created_at),
        updated_at: Set(now.into()),
        archived_at: Set(None),
        archived_by: Set(None),
    };
    let unarchived = active_model.update(&txn).await?.try_into_model()?;
    txn.commit().await?;
    Ok(unarchived)
}

pub async fn delete_by_id(db: &impl TransactionTrait, id: Id) -> Result<(), Error> {
    let txn = db.begin().await?;
    let organization_model = find_by_id(&txn, id).await?;

    let coaching_relationship_count = coaching_relationships::Entity::find()
        .filter(coaching_relationships::Column::OrganizationId.eq(id))
        .count(&txn)
        .await?;

    if coaching_relationship_count > 0 {
        let relationship_ids: Vec<Id> = coaching_relationships::Entity::find()
            .select_only()
            .column(coaching_relationships::Column::Id)
            .filter(coaching_relationships::Column::OrganizationId.eq(id))
            .into_tuple()
            .all(&txn)
            .await?;

        let coaching_session_count = coaching_sessions::Entity::find()
            .filter(coaching_sessions::Column::CoachingRelationshipId.is_in(relationship_ids))
            .count(&txn)
            .await?;

        let member_count = user_roles::Entity::find()
            .filter(user_roles::Column::OrganizationId.eq(id))
            .count(&txn)
            .await?;

        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationNotEmpty {
                coaching_relationship_count,
                coaching_session_count,
                member_count,
            },
        });
    }

    organization_model.delete(&txn).await?;
    txn.commit().await?;
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
    let mut status = StatusFilter::default();
    let mut user_id: Option<Id> = None;

    for (key, value) in params {
        match key.as_str() {
            "status" => {
                status = match value.to_lowercase().as_str() {
                    "active" => StatusFilter::Active,
                    "archived" => StatusFilter::Archived,
                    "all" => StatusFilter::All,
                    _ => {
                        return Err(Error {
                            source: None,
                            error_kind: EntityApiErrorKind::InvalidQueryTerm,
                        })
                    }
                };
            }
            "user_id" => {
                user_id = Some(uuid_parse_str(&value)?);
            }
            _ => {
                return Err(Error {
                    source: None,
                    error_kind: EntityApiErrorKind::InvalidQueryTerm,
                });
            }
        }
    }

    match user_id {
        Some(user_id) => find_by_user(db, user_id, status).await,
        None => Ok(apply_status_filter(Entity::find(), status).all(db).await?),
    }
}

pub async fn find_by_user(
    db: &impl ConnectionTrait,
    user_id: Id,
    status: StatusFilter,
) -> Result<Vec<Model>, Error> {
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
        apply_status_filter(Entity::find(), status).all(db).await?
    } else {
        // Regular users only see organizations they're explicitly assigned to
        apply_status_filter(by_user(Entity::find(), user_id).await, status)
            .all(db)
            .await?
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
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult, Transaction};

    fn test_org(name: &str, archived: bool) -> organizations::Model {
        let now = Utc::now();
        organizations::Model {
            id: Id::new_v4(),
            name: name.to_owned(),
            slug: slugify!(name),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
            archived_at: archived.then(|| now.into()),
            archived_by: archived.then(Id::new_v4),
        }
    }

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
                archived_at: None,
                archived_by: None,
            },
            organizations::Model {
                id: Id::new_v4(),
                name: "Organization One".to_owned(),
                slug: "organization-one".to_owned(),
                created_at: now.into(),
                updated_at: now.into(),
                logo: None,
                archived_at: None,
                archived_by: None,
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
        let _ = find_by_user(&db, user_id, StatusFilter::Active).await;

        // Active filter adds an "archived_at" IS NULL predicate to the org query.
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
                    r#"SELECT DISTINCT "organizations"."id", "organizations"."name", "organizations"."logo", "organizations"."slug", "organizations"."created_at", "organizations"."updated_at", "organizations"."archived_at", "organizations"."archived_by" FROM "refactor_platform"."organizations" INNER JOIN "refactor_platform"."user_roles" ON "organizations"."id" = "user_roles"."organization_id" WHERE "user_roles"."user_id" = $1 AND "organizations"."archived_at" IS NULL"#,
                    [user_id.into()]
                )
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_user_all_status_issues_no_archive_filter() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<user_roles::Model, Vec<user_roles::Model>, _>(vec![vec![]])
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let user_id = Id::new_v4();
        let _ = find_by_user(&db, user_id, StatusFilter::All).await;

        // All filter leaves the org query free of any archive predicate.
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
                    r#"SELECT DISTINCT "organizations"."id", "organizations"."name", "organizations"."logo", "organizations"."slug", "organizations"."created_at", "organizations"."updated_at", "organizations"."archived_at", "organizations"."archived_by" FROM "refactor_platform"."organizations" INNER JOIN "refactor_platform"."user_roles" ON "organizations"."id" = "user_roles"."organization_id" WHERE "user_roles"."user_id" = $1"#,
                    [user_id.into()]
                )
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn archive_sets_archived_fields() -> Result<(), Error> {
        let org = test_org("Acme", false);
        let archived = organizations::Model {
            archived_at: Some(Utc::now().into()),
            archived_by: Some(Id::new_v4()),
            ..org.clone()
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org.clone()]]) // find_by_id
            .append_query_results(vec![vec![archived.clone()]]) // update returns row
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let result = archive(&db, org.id, Id::new_v4()).await?;
        assert!(result.archived_at.is_some());
        assert!(result.archived_by.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn archive_already_archived_is_noop() -> Result<(), Error> {
        let org = test_org("Acme", true);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org.clone()]]) // find_by_id only
            .into_connection();

        let result = archive(&db, org.id, Id::new_v4()).await?;
        assert_eq!(result.id, org.id);
        assert!(result.archived_at.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn unarchive_clears_archived_fields() -> Result<(), Error> {
        let org = test_org("Acme", true);
        let cleared = organizations::Model {
            archived_at: None,
            archived_by: None,
            ..org.clone()
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org.clone()]]) // find_by_id
            .append_query_results(vec![vec![cleared.clone()]]) // update returns row
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let result = unarchive(&db, org.id).await?;
        assert!(result.archived_at.is_none());
        assert!(result.archived_by.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn unarchive_active_is_noop() -> Result<(), Error> {
        let org = test_org("Acme", false);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org.clone()]]) // find_by_id only
            .into_connection();

        let result = unarchive(&db, org.id).await?;
        assert_eq!(result.id, org.id);
        assert!(result.archived_at.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn delete_by_id_blocks_when_not_empty() {
        let org = test_org("Acme", false);
        let relationship = coaching_relationships::Model {
            id: Id::new_v4(),
            organization_id: org.id,
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            slug: "rel".to_owned(),
            created_at: Utc::now().into(),
            updated_at: Utc::now().into(),
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org.clone()]]) // find_by_id
            .append_query_results(vec![vec![maplike_count(1)]]) // rel count
            .append_query_results(vec![vec![relationship.clone()]]) // rel ids
            .append_query_results(vec![vec![maplike_count(2)]]) // session count
            .append_query_results(vec![vec![maplike_count(3)]]) // member count
            .into_connection();

        let result = delete_by_id(&db, org.id).await;
        let err = result.unwrap_err();
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::OrganizationNotEmpty {
                coaching_relationship_count: 1,
                coaching_session_count: 2,
                member_count: 3,
            }
        ));
    }

    #[tokio::test]
    async fn delete_by_id_deletes_when_empty() -> Result<(), Error> {
        let org = test_org("Acme", false);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org.clone()]]) // find_by_id
            .append_query_results(vec![vec![maplike_count(0)]]) // rel count == 0
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        delete_by_id(&db, org.id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn create_rejects_existing_name() {
        let existing = test_org("Acme", false);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]]) // name pre-check finds a row
            .into_connection();

        let new_model = test_org("Acme", false);
        let result = create(&db, new_model).await;
        let err = result.unwrap_err();
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::OrganizationNameTaken { .. }
        ));
    }

    #[tokio::test]
    async fn update_rejects_name_held_by_other_org() {
        let target = test_org("Acme", false);
        let other = test_org("Beta", false);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![target.clone()]]) // find_by_id
            .append_query_results(vec![vec![other.clone()]]) // name pre-check finds another org
            .into_connection();

        let renamed = organizations::Model {
            name: "Beta".to_owned(),
            ..target.clone()
        };
        let result = update(&db, target.id, renamed).await;
        let err = result.unwrap_err();
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::OrganizationNameTaken { .. }
        ));
    }

    // Helper to produce a `.count()` scalar result row.
    fn maplike_count(n: i64) -> std::collections::BTreeMap<String, sea_orm::Value> {
        let mut m = std::collections::BTreeMap::new();
        m.insert("num_items".to_owned(), sea_orm::Value::BigInt(Some(n)));
        m
    }
}
