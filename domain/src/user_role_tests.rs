use super::*;
use crate::error::{DomainErrorKind, EntityErrorKind, InternalErrorKind};
use crate::Actor;
use crate::{coaching_relationships, organizations, user_roles};
use sea_orm::{DatabaseBackend, DbErr, MockDatabase, MockExecResult};
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

/// The audit row a grant or removal appends.
fn role_change(
    actor_user_id: Id,
    target_user_id: Id,
    organization_id: Id,
    previous_role: Option<Role>,
    new_role: Option<Role>,
) -> entity::user_role_changes::Model {
    entity::user_role_changes::Model {
        id: Id::new_v4(),
        actor_user_id: Some(actor_user_id),
        target_user_id,
        organization_id: Some(organization_id),
        previous_role,
        new_role,
        changed_at: chrono::Utc::now().into(),
    }
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

/// Every statement the mock saw, in order, across transactions.
fn full_statements(db: sea_orm::DatabaseConnection) -> Vec<sea_orm::Statement> {
    db.into_transaction_log()
        .iter()
        .flat_map(|transaction| transaction.statements().to_vec())
        .collect()
}

/// The one audit insert, with its position among all statements.
fn audit_insert(statements: &[sea_orm::Statement]) -> (usize, Vec<sea_orm::Value>) {
    let matches: Vec<usize> = statements
        .iter()
        .enumerate()
        .filter(|(_, statement)| {
            statement.sql.contains("INSERT") && statement.sql.contains("user_role_changes")
        })
        .map(|(index, _)| index)
        .collect();

    assert_eq!(matches.len(), 1, "expected exactly one audit insert");
    let index = matches[0];
    let values = statements[index]
        .values
        .as_ref()
        .map(|values| values.0.clone())
        .unwrap_or_default();
    (index, values)
}

fn role_value(role: Role) -> sea_orm::Value {
    sea_orm::Value::String(Some(Box::new(role.to_string())))
}

#[tokio::test]
async fn attach_to_organization_returns_the_user_scoped_to_the_target_org() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let actor_user_id = Id::new_v4();
    let target_role = user_role_model(user_id, Some(organization_id), Role::User);
    let other_role = user_role_model(user_id, Some(Id::new_v4()), Role::Admin);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results([[user(user_id, vec![])]])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([[target_role.clone()]])
        .append_query_results([[role_change(
            actor_user_id,
            user_id,
            organization_id,
            None,
            Some(Role::User),
        )]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![
            (user(user_id, vec![]), Some(target_role.clone())),
            (user(user_id, vec![]), Some(other_role)),
        ]])
        .into_connection();

    let attached = attach_to_organization(
        &db,
        Actor::new(actor_user_id),
        organization_id,
        user_id,
        Role::User,
        None,
    )
    .await?;

    assert_eq!(attached.id, user_id);
    assert_eq!(attached.roles.len(), 1);
    assert_eq!(attached.roles[0].organization_id, Some(organization_id));

    // The lock is only worth taking if both racing paths take it; account
    // deletion holds up its half in delete_still_cascades_for_a_single_organization_user.
    let sql = statements(db);
    assert!(
        sql.iter()
            .any(|statement| statement.contains("FOR UPDATE") && statement.contains(r#""users""#)),
        "the new membership must land under a lock on the user row: {sql:?}"
    );

    Ok(())
}

/// The audit row is only trustworthy if it cannot be dropped independently of the
/// grant, so it has to land before the commit.
#[tokio::test]
async fn attach_to_organization_audits_the_grant_inside_the_transaction() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let actor_user_id = Id::new_v4();
    let target_role = user_role_model(user_id, Some(organization_id), Role::Admin);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results([[user(user_id, vec![])]])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([[target_role.clone()]])
        .append_query_results([[role_change(
            actor_user_id,
            user_id,
            organization_id,
            None,
            Some(Role::Admin),
        )]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
            user(user_id, vec![]),
            Some(target_role),
        )]])
        .into_connection();

    attach_to_organization(
        &db,
        Actor::new(actor_user_id),
        organization_id,
        user_id,
        Role::Admin,
        None,
    )
    .await?;

    let statements = full_statements(db);
    let (index, values) = audit_insert(&statements);

    let commit = statements
        .iter()
        .position(|statement| statement.sql == "COMMIT")
        .expect("the grant must commit");
    assert!(index < commit, "the audit row must precede the commit");

    assert!(values.contains(&sea_orm::Value::Uuid(Some(Box::new(actor_user_id)))));
    assert!(values.contains(&sea_orm::Value::Uuid(Some(Box::new(user_id)))));
    // A first grant: no previous role, the granted role as the new one.
    assert!(values.contains(&sea_orm::Value::String(None)));
    assert!(values.contains(&role_value(Role::Admin)));

    Ok(())
}

/// Mocks every query `attach_to_organization` runs up to and including the role
/// insert, then the lookups `coaching_relationship::create` runs before its own
/// insert.
fn mock_attach_with_coach(
    actor_user_id: Id,
    organization_id: Id,
    user_id: Id,
    coach_id: Id,
) -> MockDatabase {
    let membership = [organization(organization_id, false)];
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results([[user(user_id, vec![])]])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::User)]])
        .append_query_results([[role_change(
            actor_user_id,
            user_id,
            organization_id,
            None,
            Some(Role::User),
        )]])
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([
            vec![(user(coach_id, vec![]), None)],
            vec![(user(user_id, vec![]), None)],
        ])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([membership.clone()])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([membership])
        .append_query_results([Vec::<coaching_relationships::Model>::new()])
}

#[tokio::test]
async fn attach_to_organization_commits_the_role_and_the_coach_together() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let actor_user_id = Id::new_v4();
    let now = chrono::Utc::now();
    let target_role = user_role_model(user_id, Some(organization_id), Role::User);

    let db = mock_attach_with_coach(actor_user_id, organization_id, user_id, coach_id)
        .append_query_results([[coaching_relationships::Model {
            id: Id::new_v4(),
            organization_id,
            coach_id,
            coachee_id: user_id,
            slug: "member-member".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
        }]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
            user(user_id, vec![]),
            Some(target_role),
        )]])
        .into_connection();

    attach_to_organization(
        &db,
        Actor::new(actor_user_id),
        organization_id,
        user_id,
        Role::User,
        Some(coach_id),
    )
    .await?;

    let sql = statements(db);
    let inserts: Vec<&String> = sql.iter().filter(|s| s.contains("INSERT")).collect();
    assert_eq!(inserts.len(), 3, "{sql:?}");
    assert!(inserts.iter().any(|s| s.contains("user_roles")));
    assert!(inserts.iter().any(|s| s.contains("user_role_changes")));
    assert!(inserts.iter().any(|s| s.contains("coaching_relationships")));
    assert_eq!(sql.iter().filter(|s| s.as_str() == "COMMIT").count(), 1);
    assert!(!sql.iter().any(|s| s == "ROLLBACK"), "{sql:?}");

    Ok(())
}

#[tokio::test]
async fn attach_to_organization_rolls_back_the_role_when_the_coach_assignment_fails() {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let actor_user_id = Id::new_v4();

    let db = mock_attach_with_coach(actor_user_id, organization_id, user_id, coach_id)
        .append_query_errors([DbErr::Custom("relationship insert failed".to_owned())])
        .into_connection();

    attach_to_organization(
        &db,
        Actor::new(actor_user_id),
        organization_id,
        user_id,
        Role::User,
        Some(coach_id),
    )
    .await
    .expect_err("expected the failed coach assignment to fail the whole call");

    let sql = statements(db);
    assert!(
        sql.iter()
            .any(|s| s.contains("INSERT") && s.contains("coaching_relationships")),
        "the failure must be the relationship insert, not an earlier lookup: {sql:?}"
    );
    assert!(sql.iter().any(|s| s == "ROLLBACK"), "{sql:?}");
    assert!(
        !sql.iter().any(|s| s == "COMMIT"),
        "the role row must not survive a failed coach assignment: {sql:?}"
    );
}

#[tokio::test]
async fn attach_to_organization_rejects_an_archived_organization() {
    let organization_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, true)]])
        .into_connection();

    let error = attach_to_organization(
        &db,
        Actor::new(Id::new_v4()),
        organization_id,
        Id::new_v4(),
        Role::User,
        None,
    )
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
        .append_query_results([[user(user_id, vec![])]])
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::User)]])
        .into_connection();

    let error = attach_to_organization(
        &db,
        Actor::new(Id::new_v4()),
        organization_id,
        user_id,
        Role::Admin,
        None,
    )
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
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::Admin)]])
        .into_connection();

    let error = remove_from_organization(&db, Actor::new(Id::new_v4()), organization_id, user_id)
        .await
        .expect_err("expected last-admin rejection");

    assert_eq!(
        entity_error_kind(&error),
        &EntityErrorKind::LastOrganizationAdmin { organization_id }
    );

    let sql = statements(db);
    assert!(
        !sql.iter().any(|sql| sql.contains("DELETE")),
        "the last admin must not be deleted"
    );
    // Without the row lock two concurrent removals both see two admins and both
    // commit, leaving the organization with none.
    assert!(
        sql.iter()
            .any(|sql| sql.contains("user_roles") && sql.contains("FOR UPDATE")),
        "the admin count must lock the rows it counts: {sql:?}"
    );
}

#[tokio::test]
async fn remove_from_organization_succeeds_when_the_member_has_sessions() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let actor_user_id = Id::new_v4();

    // The member's coaching history no longer factors into removal, so the
    // membership lookup and the one delete are all that run.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::User)]])
        .append_exec_results([exec_result(1)])
        .append_query_results([[role_change(
            actor_user_id,
            user_id,
            organization_id,
            Some(Role::User),
            None,
        )]])
        .into_connection();

    remove_from_organization(&db, Actor::new(actor_user_id), organization_id, user_id).await?;

    // The coaching history is the record of work done; removal revokes access to
    // it rather than destroying it.
    assert!(
        !statements(db)
            .iter()
            .any(|sql| sql.contains("DELETE") && sql.contains("coaching_")),
        "removal must not delete the member's coaching history"
    );

    Ok(())
}

#[tokio::test]
async fn remove_from_organization_deletes_only_the_target_org_membership() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let actor_user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::User)]])
        .append_exec_results([exec_result(1)])
        .append_query_results([[role_change(
            actor_user_id,
            user_id,
            organization_id,
            Some(Role::User),
            None,
        )]])
        .into_connection();

    remove_from_organization(&db, Actor::new(actor_user_id), organization_id, user_id).await?;

    let deletes: Vec<String> = statements(db)
        .into_iter()
        .filter(|sql| sql.contains("DELETE"))
        .collect();

    // Scoped by primary key, which pins the removal to the one membership row
    // more tightly than filtering on organization_id did.
    assert_eq!(deletes.len(), 1, "{deletes:?}");
    assert!(deletes.iter().all(|sql| sql.contains(r#""id" = $1"#)));
    assert!(deletes.iter().any(|sql| sql.contains("user_roles")));

    Ok(())
}

/// A removal records the role that was lost, which is the whole point: after the
/// delete the `user_roles` row no longer says the user was ever an admin.
#[tokio::test]
async fn remove_from_organization_audits_the_removed_role_inside_the_transaction(
) -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let actor_user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::Admin)]])
        .append_query_results([vec![
            user_role_model(user_id, Some(organization_id), Role::Admin),
            user_role_model(Id::new_v4(), Some(organization_id), Role::Admin),
        ]])
        .append_exec_results([exec_result(1)])
        .append_query_results([[role_change(
            actor_user_id,
            user_id,
            organization_id,
            Some(Role::Admin),
            None,
        )]])
        .into_connection();

    let removed =
        remove_from_organization(&db, Actor::new(actor_user_id), organization_id, user_id).await?;
    assert_eq!(removed, Role::Admin);

    let statements = full_statements(db);
    let (index, values) = audit_insert(&statements);

    let delete = statements
        .iter()
        .position(|statement| statement.sql.contains("DELETE"))
        .expect("the membership must be deleted");
    let commit = statements
        .iter()
        .position(|statement| statement.sql == "COMMIT")
        .expect("the removal must commit");
    assert!(delete < index && index < commit);

    assert!(values.contains(&role_value(Role::Admin)));
    // A removal leaves no new role.
    assert!(values.contains(&sea_orm::Value::String(None)));

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
        .append_query_results([[user(Id::new_v4(), vec![])]])
        .append_query_results([vec![count_row(2)]])
        .into_connection();

    let error = crate::user::delete(&db, Actor::new(Id::new_v4()), Id::new_v4())
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
async fn delete_refuses_when_the_user_is_an_organizations_last_admin() {
    let organization_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user(Id::new_v4(), vec![])]])
        .append_query_results([vec![count_row(1)]])
        .append_query_results([vec![id_row("organization_id", organization_id)]])
        .append_query_results([[user_role_model(
            Id::new_v4(),
            Some(organization_id),
            Role::Admin,
        )]])
        .into_connection();

    let error = crate::user::delete(&db, Actor::new(Id::new_v4()), Id::new_v4())
        .await
        .expect_err("expected last-admin rejection");

    assert_eq!(
        entity_error_kind(&error),
        &EntityErrorKind::LastOrganizationAdmin { organization_id }
    );
    // Deleting the account drops its role rows, so it must not be allowed to
    // leave the organization unadministrable by that back door.
    assert!(
        !statements(db).iter().any(|sql| sql.contains("DELETE")),
        "nothing may be deleted when the account is an organization's last admin"
    );
}

#[tokio::test]
async fn delete_still_cascades_for_a_single_organization_user() -> Result<(), Error> {
    let actor_id = Id::new_v4();
    let target_id = Id::new_v4();
    let organization_id = Id::new_v4();
    let destroyed = user_role_model(target_id, Some(organization_id), Role::User);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user(target_id, vec![])]])
        .append_query_results([vec![count_row(1)]])
        // Administers nothing, so the last-admin guard has nothing to check.
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        // The roles the delete is about to destroy, read before they are gone.
        .append_query_results([[destroyed.clone()]])
        .append_query_results([[role_change(
            actor_id,
            target_id,
            organization_id,
            Some(Role::User),
            None,
        )]])
        .append_exec_results([exec_result(1), exec_result(1), exec_result(1)])
        .into_connection();

    let destroyed_roles = crate::user::delete(&db, Actor::new(actor_id), target_id).await?;
    assert_eq!(destroyed_roles.len(), 1);

    let sql = statements(db);

    // Without the lock a membership can commit between the guards and the
    // delete, and be destroyed unchecked. It must precede the guards, not merely
    // appear somewhere in the transaction.
    let lock = sql
        .iter()
        .position(|statement| statement.contains("FOR UPDATE") && statement.contains(r#""users""#));
    let first_guard = sql
        .iter()
        .position(|statement| statement.contains("COUNT(*)"));
    assert!(
        matches!((lock, first_guard), (Some(lock), Some(guard)) if lock < guard),
        "the user row must be locked before its memberships are counted: {sql:?}"
    );

    let deletes: Vec<String> = sql
        .into_iter()
        .filter(|statement| statement.contains("DELETE"))
        .collect();

    assert_eq!(deletes.len(), 3, "{deletes:?}");
    assert!(deletes
        .iter()
        .any(|sql| sql.contains("coaching_relationships")));
    assert!(deletes.iter().any(|sql| sql.contains("user_roles")));
    assert!(deletes.iter().any(|sql| sql.contains(r#""users""#)));

    Ok(())
}

/// Account deletion destroys every role the user held, so each one owes an audit
/// row. Without this the only path that can erase a global SuperAdmin grant leaves
/// no trace of what was taken away.
#[tokio::test]
async fn delete_audits_every_destroyed_role_including_a_global_super_admin() -> Result<(), Error> {
    let actor_id = Id::new_v4();
    let target_id = Id::new_v4();
    let organization_id = Id::new_v4();
    let org_role = user_role_model(target_id, Some(organization_id), Role::User);
    let global_role = user_role_model(target_id, None, Role::SuperAdmin);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user(target_id, vec![])]])
        .append_query_results([vec![count_row(1)]])
        .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
        .append_query_results([[org_role.clone(), global_role.clone()]])
        .append_query_results([[role_change(
            actor_id,
            target_id,
            organization_id,
            Some(Role::User),
            None,
        )]])
        .append_query_results([[role_change(
            actor_id,
            target_id,
            organization_id,
            Some(Role::SuperAdmin),
            None,
        )]])
        .append_exec_results([exec_result(1), exec_result(1), exec_result(1)])
        .into_connection();

    let destroyed_roles = crate::user::delete(&db, Actor::new(actor_id), target_id).await?;
    assert_eq!(destroyed_roles.len(), 2);

    let sql = statements(db);
    let audits: Vec<usize> = sql
        .iter()
        .enumerate()
        .filter(|(_, statement)| {
            statement.contains("INSERT") && statement.contains("user_role_changes")
        })
        .map(|(index, _)| index)
        .collect();
    let commit = sql
        .iter()
        .position(|statement| statement.contains("COMMIT"))
        .expect("the transaction must commit");

    assert_eq!(audits.len(), 2, "one audit row per destroyed role: {sql:?}");
    assert!(
        audits.iter().all(|index| *index < commit),
        "audit rows must precede the commit: {sql:?}"
    );

    Ok(())
}

/// The org-admin half of the lookup's `super_admin || shares_organization`
/// condition. The super-admin test only covers the first disjunct.
#[tokio::test]
async fn lookup_by_email_scoped_returns_the_user_for_an_admin_of_a_shared_organization(
) -> Result<(), Error> {
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
    let target_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
            user(target_id, vec![]),
            None,
        )]])
        .append_query_results([vec![id_row("organization_id", organization_id)]])
        .append_query_results([vec![id_row("id", Id::new_v4())]])
        .into_connection();

    let results = lookup_by_email_scoped(&db, &requester, "member@test.com").await?;

    assert_eq!(
        results.len(),
        1,
        "an admin of a shared org may see the user"
    );
    assert_eq!(results[0].id, target_id);

    Ok(())
}

/// Documented in the function's `# Errors` section but previously unexercised.
#[tokio::test]
async fn remove_from_organization_rejects_a_non_member() {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<user_roles::Model>::new()])
        .into_connection();

    let error = remove_from_organization(&db, Actor::new(Id::new_v4()), Id::new_v4(), Id::new_v4())
        .await
        .expect_err("expected a not-found rejection");

    assert_eq!(entity_error_kind(&error), &EntityErrorKind::NotFound);
    assert!(
        !statements(db).iter().any(|sql| sql.contains("DELETE")),
        "a non-member must not trigger any delete"
    );
}

/// Documented in the function's `# Errors` section but previously unexercised.
#[tokio::test]
async fn attach_to_organization_rejects_a_missing_organization() {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<organizations::Model>::new()])
        .into_connection();

    let error = attach_to_organization(
        &db,
        Actor::new(Id::new_v4()),
        Id::new_v4(),
        Id::new_v4(),
        Role::User,
        None,
    )
    .await
    .expect_err("expected a not-found rejection");

    assert_eq!(entity_error_kind(&error), &EntityErrorKind::NotFound);
    assert!(
        !statements(db).iter().any(|sql| sql.contains("INSERT")),
        "a missing organization must not insert a role"
    );
}

/// Every query `update_role_in_organization` runs, in order, for a change that is
/// allowed to proceed. `admins` is the locked admin count the demote guard reads,
/// and is left empty on the promote and no-op paths where the guard never runs.
fn mock_update_role(
    actor_user_id: Id,
    organization_id: Id,
    user_id: Id,
    current: Role,
    requested: Role,
    admins: Vec<user_roles::Model>,
    returned_roles: Vec<user_roles::Model>,
) -> MockDatabase {
    let mut updated = user_role_model(user_id, Some(organization_id), current.clone());
    updated.role = requested.clone();

    let mut mock = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results([[user(user_id, vec![])]])
        .append_query_results([[user_role_model(
            user_id,
            Some(organization_id),
            current.clone(),
        )]]);

    if !admins.is_empty() {
        mock = mock.append_query_results([admins]);
    }

    if current != requested {
        mock = mock
            .append_query_results([[updated]])
            .append_query_results([[role_change(
                actor_user_id,
                user_id,
                organization_id,
                Some(current),
                Some(requested),
            )]]);
    }

    mock.append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([returned_roles
        .into_iter()
        .map(|role| (user(user_id, vec![]), Some(role)))
        .collect::<Vec<_>>()])
}

/// I-6. Read committed lets two concurrent demotions both observe two admins and
/// both commit, leaving the organization with none. The guard is only sound if the
/// rows it counts are locked, so the mechanism is asserted rather than the outcome:
/// swapping the locking count for a plain `COUNT(*)` still looks correct
/// single-threaded, and a `MockDatabase` cannot interleave transactions to catch it.
#[tokio::test]
async fn update_role_in_organization_refuses_to_demote_the_last_admin() {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results([[user(user_id, vec![])]])
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::Admin)]])
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::Admin)]])
        .into_connection();

    let error = update_role_in_organization(
        &db,
        Actor::new(Id::new_v4()),
        organization_id,
        user_id,
        Role::User,
    )
    .await
    .expect_err("expected last-admin rejection");

    assert_eq!(
        entity_error_kind(&error),
        &EntityErrorKind::LastOrganizationAdmin { organization_id }
    );

    let sql = statements(db);
    assert!(
        !sql.iter().any(|sql| sql.starts_with("UPDATE")),
        "the last admin must not be demoted: {sql:?}"
    );
    assert!(
        sql.iter()
            .any(|sql| sql.contains("user_roles") && sql.contains("FOR UPDATE")),
        "the admin count must lock the rows it counts: {sql:?}"
    );
}

/// The same call that is refused above succeeds once a second admin exists, so the
/// guard is reading live state rather than refusing every demotion.
#[tokio::test]
async fn update_role_in_organization_demotes_an_admin_when_another_remains() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let actor_user_id = Id::new_v4();
    let demoted = user_role_model(user_id, Some(organization_id), Role::User);

    let db = mock_update_role(
        actor_user_id,
        organization_id,
        user_id,
        Role::Admin,
        Role::User,
        vec![
            user_role_model(user_id, Some(organization_id), Role::Admin),
            user_role_model(Id::new_v4(), Some(organization_id), Role::Admin),
        ],
        vec![demoted],
    )
    .into_connection();

    let updated = update_role_in_organization(
        &db,
        Actor::new(actor_user_id),
        organization_id,
        user_id,
        Role::User,
    )
    .await?;

    assert_eq!(updated.roles.len(), 1);
    assert_eq!(updated.roles[0].role, Role::User);

    Ok(())
}

/// I-8. A role change that is not recorded is indistinguishable from one that never
/// happened, and `user_roles` keeps no history of its own. The audit row has to land
/// inside the transaction, or a partial failure leaves the grant without its trail.
#[tokio::test]
async fn update_role_in_organization_audits_the_transition_inside_the_transaction(
) -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let actor_user_id = Id::new_v4();
    let promoted = user_role_model(user_id, Some(organization_id), Role::Admin);

    let db = mock_update_role(
        actor_user_id,
        organization_id,
        user_id,
        Role::User,
        Role::Admin,
        vec![],
        vec![promoted],
    )
    .into_connection();

    update_role_in_organization(
        &db,
        Actor::new(actor_user_id),
        organization_id,
        user_id,
        Role::Admin,
    )
    .await?;

    let statements = full_statements(db);
    let (index, values) = audit_insert(&statements);

    let update = statements
        .iter()
        .position(|statement| statement.sql.starts_with("UPDATE"))
        .expect("the membership must be updated");
    let commit = statements
        .iter()
        .position(|statement| statement.sql == "COMMIT")
        .expect("the change must commit");
    assert!(
        update < index && index < commit,
        "the audit row must follow the update and precede the commit"
    );

    assert!(values.contains(&sea_orm::Value::Uuid(Some(Box::new(actor_user_id)))));
    assert!(values.contains(&sea_orm::Value::Uuid(Some(Box::new(user_id)))));
    // A change records both ends of the transition; neither side is null.
    assert!(values.contains(&role_value(Role::User)));
    assert!(values.contains(&role_value(Role::Admin)));
    assert!(
        !values.contains(&sea_orm::Value::String(None)),
        "a change is neither a first grant nor a removal: {values:?}"
    );

    Ok(())
}

/// I-7. The response must not disclose which other organizations the target belongs
/// to. That is the same fact the `UserBelongsToMultipleOrganizations` handler
/// deliberately withholds, so dropping the scoping call would leak it here instead.
#[tokio::test]
async fn update_role_in_organization_scopes_the_returned_roles_to_the_target_org(
) -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let actor_user_id = Id::new_v4();
    let in_scope = user_role_model(user_id, Some(organization_id), Role::Admin);
    let elsewhere = user_role_model(user_id, Some(Id::new_v4()), Role::Admin);

    let db = mock_update_role(
        actor_user_id,
        organization_id,
        user_id,
        Role::User,
        Role::Admin,
        vec![],
        vec![in_scope.clone(), elsewhere],
    )
    .into_connection();

    let updated = update_role_in_organization(
        &db,
        Actor::new(actor_user_id),
        organization_id,
        user_id,
        Role::Admin,
    )
    .await?;

    let organizations: Vec<Option<Id>> = updated
        .roles
        .iter()
        .map(|role| role.organization_id)
        .collect();
    assert_eq!(
        organizations,
        vec![Some(organization_id)],
        "only the path organization may appear in the response"
    );

    Ok(())
}

/// Setting the role a member already holds is a no-op, not an error, so the endpoint
/// is idempotent. It must also be free: an UPDATE would move `updated_at` and write
/// an audit row for a change that did not happen, and the admin lock would be taken
/// for nothing.
#[tokio::test]
async fn update_role_in_organization_writes_nothing_when_the_role_is_unchanged() -> Result<(), Error>
{
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let unchanged = user_role_model(user_id, Some(organization_id), Role::Admin);

    let db = mock_update_role(
        Id::new_v4(),
        organization_id,
        user_id,
        Role::Admin,
        Role::Admin,
        vec![],
        vec![unchanged],
    )
    .into_connection();

    update_role_in_organization(
        &db,
        Actor::new(Id::new_v4()),
        organization_id,
        user_id,
        Role::Admin,
    )
    .await?;

    let sql = statements(db);
    assert!(
        !sql.iter().any(|sql| sql.starts_with("UPDATE")),
        "an unchanged role must not be rewritten: {sql:?}"
    );
    assert!(
        !sql.iter().any(|sql| sql.contains("user_role_changes")),
        "an unchanged role is not a change to audit: {sql:?}"
    );
    assert!(
        !sql.iter()
            .any(|sql| sql.contains("user_roles") && sql.contains("FOR UPDATE")),
        "a no-op must not lock the organization's admin rows: {sql:?}"
    );

    Ok(())
}

/// A member the caller may not see and a member who does not exist are the same
/// empty lookup, so both answer `NotFound` and the endpoint cannot be used to probe
/// which one it was.
#[tokio::test]
async fn update_role_in_organization_rejects_a_non_member() {
    let organization_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results([[user(Id::new_v4(), vec![])]])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .into_connection();

    let error = update_role_in_organization(
        &db,
        Actor::new(Id::new_v4()),
        organization_id,
        Id::new_v4(),
        Role::Admin,
    )
    .await
    .expect_err("expected a not-found rejection");

    assert_eq!(entity_error_kind(&error), &EntityErrorKind::NotFound);
    assert!(
        !statements(db).iter().any(|sql| sql.starts_with("UPDATE")),
        "a non-member must not be written"
    );
}

/// Archiving is a write freeze. The refusal has to come before the membership is
/// read, or an archived organization still discloses who belongs to it.
#[tokio::test]
async fn update_role_in_organization_rejects_an_archived_organization() {
    let organization_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, true)]])
        .into_connection();

    let error = update_role_in_organization(
        &db,
        Actor::new(Id::new_v4()),
        organization_id,
        Id::new_v4(),
        Role::Admin,
    )
    .await
    .expect_err("expected an archived-organization rejection");

    assert_eq!(
        entity_error_kind(&error),
        &EntityErrorKind::OrganizationArchived
    );
    assert!(
        !statements(db).iter().any(|sql| sql.contains("user_roles")),
        "the freeze must be checked before the membership is read"
    );
}

/// C-3. A concurrent account deletion locks the users row and then removes every
/// membership. Without taking the same lock this path can read a membership that is
/// gone by the time it writes, and a zero-row update surfaces as a 500 rather than
/// the 404 the caller should get.
#[tokio::test]
async fn update_role_in_organization_locks_the_user_row() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();
    let promoted = user_role_model(user_id, Some(organization_id), Role::Admin);

    let db = mock_update_role(
        Id::new_v4(),
        organization_id,
        user_id,
        Role::User,
        Role::Admin,
        vec![],
        vec![promoted],
    )
    .into_connection();

    update_role_in_organization(
        &db,
        Actor::new(Id::new_v4()),
        organization_id,
        user_id,
        Role::Admin,
    )
    .await?;

    let sql = statements(db);
    let lock = sql
        .iter()
        .position(|sql| sql.contains(r#""users""#) && sql.contains("FOR UPDATE"))
        .expect("the user row must be locked");
    let membership = sql
        .iter()
        .position(|sql| sql.starts_with("SELECT") && sql.contains("user_roles"))
        .expect("the membership must be read");
    // Taken before the membership is read, and before the admin count, so the lock
    // order matches account deletion's users-then-user_roles and cannot deadlock.
    assert!(lock < membership, "the lock must precede the read: {sql:?}");

    Ok(())
}

#[tokio::test]
async fn find_role_in_organization_returns_the_membership() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[user_role_model(user_id, Some(organization_id), Role::Admin)]])
        .into_connection();

    let membership = find_role_in_organization(&db, organization_id, user_id).await?;

    assert_eq!(membership.role, Role::Admin);
    assert_eq!(membership.organization_id, Some(organization_id));

    Ok(())
}

/// The read is scoped to one organization, so a user with roles elsewhere is
/// reported as absent here rather than having those roles disclosed.
#[tokio::test]
async fn find_role_in_organization_rejects_a_non_member() {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<user_roles::Model>::new()])
        .into_connection();

    let error = find_role_in_organization(&db, Id::new_v4(), Id::new_v4())
        .await
        .expect_err("expected a not-found rejection");

    assert_eq!(entity_error_kind(&error), &EntityErrorKind::NotFound);
}
