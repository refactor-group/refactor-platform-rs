use super::error::{EntityApiErrorKind, Error};
use crate::{organization::Entity, uuid_parse_str};
use chrono::Utc;
use entity::{
    coaching_relationships, coaching_sessions, organizations::*, prelude::Organizations, roles,
    user_roles, Id,
};
use sea_orm::{
    entity::prelude::*, ActiveValue::Set, ConnectionTrait, IntoActiveModel, JoinType, QuerySelect,
    SqlErr, TransactionTrait, TryIntoModel,
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

/// Upper bound on organization name length. The column is an unbounded `varchar`,
/// so this cap is enforced here rather than by the database.
pub const MAX_ORG_NAME_LEN: usize = 255;

/// Reject an empty/whitespace-only or over-length organization name, returning
/// the trimmed name on success. Counts characters (not bytes) for the cap.
fn validate_name(name: &str) -> Result<String, Error> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationNameInvalid {
                message: "Organization name must not be empty.".to_string(),
            },
        });
    }
    let length = trimmed.chars().count();
    if length > MAX_ORG_NAME_LEN {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationNameInvalid {
                message: format!(
                    "Organization name must be at most {MAX_ORG_NAME_LEN} characters (got {length})."
                ),
            },
        });
    }
    Ok(trimmed.to_string())
}

pub async fn create(db: &impl TransactionTrait, organization_model: Model) -> Result<Model, Error> {
    debug!("New Organization Model to be inserted: {organization_model:?}");

    let name = validate_name(&organization_model.name)?;
    let slug = slugify!(name.as_str());

    let txn = db.begin().await?;
    let now = Utc::now();

    // Name/slug-collision pre-check. slug derives from name but is independently
    // unique, so distinct names can still collide on slug.
    if Entity::find()
        .filter(Column::Name.eq(&name).or(Column::Slug.eq(&slug)))
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
        slug: Set(slug.clone()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    let inserted = match organization_active_model.insert(&txn).await {
        Ok(model) => model,
        Err(insert_err) => {
            // Race backstop: inspect the error, not a re-query (the failed INSERT
            // has aborted this txn, so any further query on it would error).
            if matches!(
                insert_err.sql_err(),
                Some(SqlErr::UniqueConstraintViolation(_))
            ) {
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
    let name = validate_name(&model.name)?;
    // Re-derive the slug from the new name so it never goes stale (a stale slug
    // keeps its UNIQUE index entry and would block a later create of the old name).
    let slug = slugify!(name.as_str());

    let txn = db.begin().await?;
    let organization = find_by_id(&txn, id).await?;

    // Name/slug-collision pre-check against OTHER orgs (rename re-slugs, so either can collide).
    if Entity::find()
        .filter(Column::Id.ne(id))
        .filter(Column::Name.eq(&name).or(Column::Slug.eq(&slug)))
        .one(&txn)
        .await?
        .is_some()
    {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationNameTaken { name },
        });
    }

    let mut active_model = organization.into_active_model();
    active_model.name = Set(name);
    active_model.logo = Set(model.logo);
    active_model.slug = Set(slug);
    active_model.updated_at = Set(Utc::now().into());
    let updated = active_model.update(&txn).await?.try_into_model()?;
    txn.commit().await?;
    Ok(updated)
}

/// Archive an organization (idempotent). Re-archiving is a no-op that avoids
/// timestamp churn.
pub async fn archive(db: &impl TransactionTrait, id: Id, archived_by: Id) -> Result<Model, Error> {
    set_archive_state(db, id, true, Some(archived_by)).await
}

/// Unarchive an organization (idempotent). Unarchiving an active org is a no-op.
pub async fn unarchive(db: &impl TransactionTrait, id: Id) -> Result<Model, Error> {
    set_archive_state(db, id, false, None).await
}

/// Set (archive) or clear (unarchive) the archive marker. Idempotent: returns the
/// org untouched when already in the target state, so re-runs don't churn `updated_at`.
async fn set_archive_state(
    db: &impl TransactionTrait,
    id: Id,
    archived: bool,
    archived_by: Option<Id>,
) -> Result<Model, Error> {
    let txn = db.begin().await?;
    let organization = find_by_id(&txn, id).await?;

    if organization.archived_at.is_some() == archived {
        txn.commit().await?;
        return Ok(organization);
    }

    let now = Utc::now();
    let mut active_model = organization.into_active_model();
    active_model.updated_at = Set(now.into());
    active_model.archived_at = Set(archived.then(|| now.into()));
    active_model.archived_by = Set(archived_by);
    let updated = active_model.update(&txn).await?.try_into_model()?;
    txn.commit().await?;
    Ok(updated)
}

pub async fn delete_by_id(db: &impl TransactionTrait, id: Id) -> Result<(), Error> {
    let txn = db.begin().await?;
    let organization_model = find_by_id(&txn, id).await?;

    // An org is deletable only when empty of BOTH coaching relationships AND members
    // (user_roles). Members alone must block: their rows are ON DELETE CASCADE, so
    // deleting an org that still has members would silently drop their role grants.
    let relationship_ids: Vec<Id> = coaching_relationships::Entity::find()
        .select_only()
        .column(coaching_relationships::Column::Id)
        .filter(coaching_relationships::Column::OrganizationId.eq(id))
        .into_tuple()
        .all(&txn)
        .await?;
    let coaching_relationship_count = relationship_ids.len() as u64;

    let member_count = user_roles::Entity::find()
        .filter(user_roles::Column::OrganizationId.eq(id))
        .count(&txn)
        .await?;

    if coaching_relationship_count > 0 || member_count > 0 {
        // Sessions hang off relationships, so only worth counting when there are any.
        let coaching_session_count = if coaching_relationship_count == 0 {
            0
        } else {
            coaching_sessions::Entity::find()
                .filter(coaching_sessions::Column::CoachingRelationshipId.is_in(relationship_ids))
                .count(&txn)
                .await?
        };

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
            .append_query_results(vec![vec![relationship.clone()]]) // rel ids (len = 1)
            .append_query_results(vec![vec![maplike_count(3)]]) // member count
            .append_query_results(vec![vec![maplike_count(2)]]) // session count
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
    async fn delete_by_id_blocks_when_only_members() {
        // An org with members but zero relationships must still block (not silently
        // cascade-delete the user_roles).
        let org = test_org("Acme", false);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org.clone()]]) // find_by_id
            .append_query_results(vec![Vec::<coaching_relationships::Model>::new()]) // no rels
            .append_query_results(vec![vec![maplike_count(5)]]) // 5 members
            .into_connection();

        let result = delete_by_id(&db, org.id).await;
        assert!(matches!(
            result.unwrap_err().error_kind,
            EntityApiErrorKind::OrganizationNotEmpty {
                coaching_relationship_count: 0,
                coaching_session_count: 0,
                member_count: 5,
            }
        ));
    }

    #[tokio::test]
    async fn delete_by_id_deletes_when_empty() -> Result<(), Error> {
        let org = test_org("Acme", false);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org.clone()]]) // find_by_id
            .append_query_results(vec![Vec::<coaching_relationships::Model>::new()]) // no rels
            .append_query_results(vec![vec![maplike_count(0)]]) // no members
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
    async fn create_rejects_blank_name() {
        // Whitespace-only name is rejected before any DB round-trip.
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut model = test_org("Acme", false);
        model.name = "   ".to_string();
        let err = create(&db, model).await.unwrap_err();
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::OrganizationNameInvalid { .. }
        ));
    }

    #[tokio::test]
    async fn create_rejects_overlong_name() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut model = test_org("Acme", false);
        model.name = "x".repeat(MAX_ORG_NAME_LEN + 1);
        let err = create(&db, model).await.unwrap_err();
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::OrganizationNameInvalid { .. }
        ));
    }

    #[tokio::test]
    async fn update_rejects_blank_name() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let mut model = test_org("Acme", false);
        model.name = "".to_string();
        let err = update(&db, model.id, model.clone()).await.unwrap_err();
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::OrganizationNameInvalid { .. }
        ));
    }

    #[tokio::test]
    async fn create_pre_check_filters_on_slug_too() {
        // Distinct names can slugify to the same value, and slug is independently
        // unique. The create pre-check must filter on slug (not name alone), or a
        // slug-only collision falls through to the insert and surfaces as a 503
        // instead of OrganizationNameTaken.
        let existing = test_org("Acme Corp", false); // slug "acme-corp"
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]]) // pre-check returns the slug-colliding row
            .into_connection();

        // "Acme Corp!" -> slug "acme-corp" (same), but a different name.
        let result = create(&db, test_org("Acme Corp!", false)).await;
        assert!(matches!(
            result.unwrap_err().error_kind,
            EntityApiErrorKind::OrganizationNameTaken { .. }
        ));

        // Teeth: the emitted pre-check SQL must carry a slug predicate, not just
        // the slug column in the projection. (Debug-escapes quotes; unescape first.)
        let log = format!("{:?}", db.into_transaction_log()).replace("\\\"", "\"");
        assert!(
            log.contains(r#"OR "organizations"."slug" ="#),
            "create pre-check must filter on slug, not name alone; got: {log}"
        );
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

    #[tokio::test]
    async fn update_reslugs_on_rename() {
        // Renaming must re-derive the slug so it can't go stale (a stale slug keeps
        // its UNIQUE entry and would block a later create of the old name).
        let target = test_org("Acme", false); // slug "acme"
        let renamed_row = organizations::Model {
            name: "Acme Corp".to_owned(),
            slug: "acme-corp".to_owned(),
            ..target.clone()
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![target.clone()]]) // find_by_id
            .append_query_results(vec![Vec::<organizations::Model>::new()]) // pre-check: no collision
            .append_query_results(vec![vec![renamed_row.clone()]]) // update RETURNING
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let renamed = organizations::Model {
            name: "Acme Corp".to_owned(),
            ..target.clone()
        };
        let _ = update(&db, target.id, renamed)
            .await
            .expect("rename succeeds");

        // Teeth: the emitted UPDATE must re-set slug (old code left it Unchanged).
        let log = format!("{:?}", db.into_transaction_log()).replace("\\\"", "\"");
        let set_clause = log
            .split("UPDATE")
            .nth(1)
            .and_then(|after| after.split("RETURNING").next())
            .expect("an UPDATE ... RETURNING was issued");
        assert!(
            set_clause.contains(r#""slug" ="#),
            "rename must re-slugify (UPDATE should SET slug); got: {log}"
        );
    }

    // Helper to produce a `.count()` scalar result row.
    fn maplike_count(n: i64) -> std::collections::BTreeMap<String, sea_orm::Value> {
        let mut m = std::collections::BTreeMap::new();
        m.insert("num_items".to_owned(), sea_orm::Value::BigInt(Some(n)));
        m
    }
}
