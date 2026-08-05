use super::*;
use crate::error::{DomainErrorKind, EntityErrorKind, InternalErrorKind};
use crate::{organizations, user_roles};
use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};
use std::collections::BTreeMap;

fn organization(id: Id, archived: bool) -> organizations::Model {
    let now = chrono::Utc::now();
    organizations::Model {
        id,
        name: "Test Org".to_owned(),
        logo: None,
        slug: "test-org".to_owned(),
        created_at: now.into(),
        updated_at: now.into(),
        archived_at: archived.then(|| now.into()),
        archived_by: archived.then(Id::new_v4),
    }
}

fn user(id: Id, roles: Vec<user_roles::Model>) -> users::Model {
    let now = chrono::Utc::now();
    users::Model {
        id,
        email: "member@test.com".to_owned(),
        first_name: "Member".to_owned(),
        last_name: "User".to_owned(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        created_at: now.into(),
        updated_at: now.into(),
        role: Role::User,
        roles,
        invite_status: None,
    }
}

fn user_role_model(user_id: Id, organization_id: Option<Id>, role: Role) -> user_roles::Model {
    let now = chrono::Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        user_id,
        organization_id,
        role,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn count_row(count: i64) -> BTreeMap<String, sea_orm::Value> {
    BTreeMap::from([("num_items".to_string(), count.into())])
}

/// A single-column mock row, for the id-only selects.
fn id_row(column: &str, value: Id) -> BTreeMap<String, sea_orm::Value> {
    BTreeMap::from([(column.to_string(), value.into())])
}

fn exec_result(rows_affected: u64) -> MockExecResult {
    MockExecResult {
        last_insert_id: 0,
        rows_affected,
    }
}

/// Unwraps the entity-layer kind, failing loudly on any other error shape.
fn entity_error_kind(error: &Error) -> &EntityErrorKind {
    match &error.error_kind {
        DomainErrorKind::Internal(InternalErrorKind::Entity(kind)) => kind,
        other => panic!("expected an entity error, got {other:?}"),
    }
}

/// Every statement the mock saw, flattened across transactions.
fn statements(db: sea_orm::DatabaseConnection) -> Vec<String> {
    db.into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
        })
        .collect()
}

#[tokio::test]
async fn attach_to_organization_returns_the_user_scoped_to_the_target_org() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let target_role = user_role_model(user_id, Some(organization_id), Role::User);
    let other_role = user_role_model(user_id, Some(Id::new_v4()), Role::Admin);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
            user(user_id, vec![]),
            None,
        )]])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([[target_role.clone()]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![
            (user(user_id, vec![]), Some(target_role.clone())),
            (user(user_id, vec![]), Some(other_role)),
        ]])
        .into_connection();

    let attached = attach_to_organization(&db, organization_id, user_id, Role::User).await?;

    assert_eq!(attached.id, user_id);
    assert_eq!(attached.roles.len(), 1);
    assert_eq!(attached.roles[0].organization_id, Some(organization_id));

    Ok(())
}

#[tokio::test]
async fn attach_to_organization_rejects_an_archived_organization() {
    let organization_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, true)]])
        .into_connection();

    let error = attach_to_organization(&db, organization_id, Id::new_v4(), Role::User)
        .await
        .expect_err("expected archived-org rejection");

    assert_eq!(
        entity_error_kind(&error),
        &EntityErrorKind::OrganizationArchived
    );
    assert!(
        !statements(db).iter().any(|sql| sql.contains("INSERT")),
        "an archived organization must not insert a role row"
    );
}

#[tokio::test]
async fn attach_to_organization_rejects_an_existing_membership() {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
            user(user_id, vec![]),
            None,
        )]])
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::User)]])
        .into_connection();

    let error = attach_to_organization(&db, organization_id, user_id, Role::Admin)
        .await
        .expect_err("expected duplicate-membership rejection");

    assert_eq!(
        entity_error_kind(&error),
        &EntityErrorKind::UserAlreadyInOrganization { organization_id }
    );
}

#[tokio::test]
async fn remove_from_organization_refuses_to_remove_the_last_admin() {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::Admin)]])
        .append_query_results([vec![count_row(1)]])
        .into_connection();

    let error = remove_from_organization(&db, organization_id, user_id)
        .await
        .expect_err("expected last-admin rejection");

    assert_eq!(
        entity_error_kind(&error),
        &EntityErrorKind::LastOrganizationAdmin { organization_id }
    );
    assert!(
        !statements(db).iter().any(|sql| sql.contains("DELETE")),
        "the last admin must not be deleted"
    );
}

#[tokio::test]
async fn remove_from_organization_deletes_only_the_target_org_membership() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::User)]])
        .append_exec_results([exec_result(1), exec_result(1)])
        .into_connection();

    remove_from_organization(&db, organization_id, user_id).await?;

    let deletes: Vec<String> = statements(db)
        .into_iter()
        .filter(|sql| sql.contains("DELETE"))
        .collect();

    assert_eq!(deletes.len(), 2, "{deletes:?}");
    assert!(deletes.iter().all(|sql| sql.contains("organization_id")));
    assert!(deletes.iter().any(|sql| sql.contains("user_roles")));
    assert!(deletes
        .iter()
        .any(|sql| sql.contains("coaching_relationships")));

    Ok(())
}

#[tokio::test]
async fn lookup_by_email_scoped_returns_empty_for_an_unknown_email() -> Result<(), Error> {
    let requester = user(Id::new_v4(), vec![]);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![]])
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .into_connection();

    let results = lookup_by_email_scoped(&db, &requester, " nobody@test.com ").await?;

    assert!(results.is_empty());
    assert_eq!(statements(db).len(), 3);

    Ok(())
}

#[tokio::test]
async fn lookup_by_email_scoped_hides_a_user_outside_the_requesters_scope() -> Result<(), Error> {
    let requester = user(Id::new_v4(), vec![]);
    let target_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
            user(target_id, vec![]),
            None,
        )]])
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .into_connection();

    let results = lookup_by_email_scoped(&db, &requester, "member@test.com").await?;

    assert!(results.is_empty());
    // Same query count as the unknown-email path: the scope check runs either way,
    // so response timing cannot separate "no such email" from "not yours to see".
    assert_eq!(statements(db).len(), 3);

    Ok(())
}

#[tokio::test]
async fn lookup_by_email_scoped_returns_the_user_for_a_super_admin() -> Result<(), Error> {
    let requester_id = Id::new_v4();
    let requester = user(
        requester_id,
        vec![user_role_model(requester_id, None, Role::SuperAdmin)],
    );
    let target_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
            user(target_id, vec![]),
            None,
        )]])
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .into_connection();

    let results = lookup_by_email_scoped(&db, &requester, "member@test.com").await?;

    assert_eq!(
        results,
        vec![UserLookupResult {
            id: target_id,
            first_name: "Member".to_string(),
            last_name: "User".to_string(),
            email: "member@test.com".to_string(),
        }]
    );
    // The scope check still ran, even though the super-admin answer was already known.
    assert_eq!(statements(db).len(), 3);

    Ok(())
}

#[tokio::test]
async fn can_administer_user_is_true_for_a_super_admin() -> Result<(), Error> {
    let requester_id = Id::new_v4();
    let requester = user(
        requester_id,
        vec![user_role_model(requester_id, None, Role::SuperAdmin)],
    );

    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    assert!(can_administer_user(&db, &requester, Id::new_v4()).await?);
    assert!(
        statements(db).is_empty(),
        "a super admin needs no scope query"
    );

    Ok(())
}

#[tokio::test]
async fn can_administer_user_is_true_for_an_admin_of_a_shared_organization() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let requester_id = Id::new_v4();
    let requester = user(
        requester_id,
        vec![user_role_model(
            requester_id,
            Some(organization_id),
            Role::Admin,
        )],
    );

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![id_row("organization_id", organization_id)]])
        .append_query_results([vec![id_row("id", Id::new_v4())]])
        .into_connection();

    assert!(can_administer_user(&db, &requester, Id::new_v4()).await?);

    Ok(())
}

#[tokio::test]
async fn can_administer_user_is_false_for_a_plain_member() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let requester_id = Id::new_v4();
    let requester = user(
        requester_id,
        vec![user_role_model(
            requester_id,
            Some(organization_id),
            Role::User,
        )],
    );

    // The requester holds no Admin role, so the admin-scoped query finds nothing.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .into_connection();

    assert!(!can_administer_user(&db, &requester, Id::new_v4()).await?);

    Ok(())
}

#[tokio::test]
async fn delete_refuses_a_user_who_belongs_to_multiple_organizations() {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![count_row(2)]])
        .into_connection();

    let error = crate::user::delete(&db, Id::new_v4())
        .await
        .expect_err("expected multi-org rejection");

    assert_eq!(
        entity_error_kind(&error),
        &EntityErrorKind::UserBelongsToMultipleOrganizations {
            organization_count: 2
        }
    );
    assert!(
        !statements(db).iter().any(|sql| sql.contains("DELETE")),
        "a multi-org user must not be deleted"
    );
}

#[tokio::test]
async fn delete_still_cascades_for_a_single_organization_user() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![count_row(1)]])
        .append_exec_results([exec_result(1), exec_result(1), exec_result(1)])
        .into_connection();

    crate::user::delete(&db, Id::new_v4()).await?;

    let deletes: Vec<String> = statements(db)
        .into_iter()
        .filter(|sql| sql.contains("DELETE"))
        .collect();

    assert_eq!(deletes.len(), 3, "{deletes:?}");
    assert!(deletes
        .iter()
        .any(|sql| sql.contains("coaching_relationships")));
    assert!(deletes.iter().any(|sql| sql.contains("user_roles")));
    assert!(deletes.iter().any(|sql| sql.contains(r#""users""#)));

    Ok(())
}
