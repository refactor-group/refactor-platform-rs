use super::*;
use crate::user::scope_roles_to_organization;
use sea_orm::{DatabaseBackend, MockDatabase, Value};
use std::collections::BTreeMap;

fn role_model(user_id: Id, organization_id: Option<Id>, role: Role) -> Model {
    let now = chrono::Utc::now();
    Model {
        id: Id::new_v4(),
        user_id,
        organization_id,
        role,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn user_model(roles: Vec<Model>) -> entity::users::Model {
    let now = chrono::Utc::now();
    entity::users::Model {
        id: Id::new_v4(),
        email: "test@test.com".to_owned(),
        first_name: "Test".to_owned(),
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

/// A single-column mock row, for the id-only selects.
fn column_row(column: &str, value: Id) -> BTreeMap<String, Value> {
    BTreeMap::from([(column.to_string(), value.into())])
}

/// The row shape `count()` reads back.
fn count_row(count: i64) -> BTreeMap<String, Value> {
    BTreeMap::from([("num_items".to_string(), count.into())])
}

#[tokio::test]
async fn create_rejects_super_admin_without_touching_the_database() {
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let result = create(
        &db,
        Actor::new(Id::new_v4()),
        Id::new_v4(),
        Id::new_v4(),
        Role::SuperAdmin,
    )
    .await;

    let err = result.expect_err("expected SuperAdmin rejection");
    assert!(matches!(
        err.error_kind,
        EntityApiErrorKind::ValidationError { .. }
    ));
    assert!(
        db.into_transaction_log().is_empty(),
        "SuperAdmin must be rejected before any query runs"
    );
}

#[tokio::test]
async fn create_inserts_an_organization_scoped_role() -> Result<(), Error> {
    let user_id = Id::new_v4();
    let organization_id = Id::new_v4();
    let inserted = role_model(user_id, Some(organization_id), Role::Admin);

    let actor_user_id = Id::new_v4();
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[inserted.clone()]])
        .append_query_results([[entity::user_role_changes::Model {
            id: Id::new_v4(),
            actor_user_id: Some(actor_user_id),
            target_user_id: user_id,
            organization_id: Some(organization_id),
            previous_role: None,
            new_role: Some(Role::Admin),
            changed_at: chrono::Utc::now().into(),
        }]])
        .into_connection();

    let created = create(
        &db,
        Actor::new(actor_user_id),
        user_id,
        organization_id,
        Role::Admin,
    )
    .await?;

    assert_eq!(created.user_id, user_id);
    assert_eq!(created.organization_id, Some(organization_id));
    assert_eq!(created.role, Role::Admin);

    // The grant audits itself, so no caller can create a role without a record.
    let sql: Vec<String> = db
        .into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
        })
        .collect();
    assert!(
        sql.iter()
            .any(|statement| statement.contains("user_role_changes")),
        "create must audit the grant: {sql:?}"
    );

    Ok(())
}

/// The mapping is matched on the constraint name rather than the message text,
/// which is locale and version sensitive. A `DbErr` carrying a real `SqlErr`
/// cannot be constructed by hand, so the policy is tested at its own boundary.
/// The only guard on duplicate-membership returning 409 rather than 500. The
/// racing insert can only be refused by the database, and `MockDatabase` cannot
/// produce a real `SqlErr`, so this cannot be covered above the predicate.
#[test]
fn the_one_role_per_organization_index_is_recognised_as_a_membership_conflict() {
    assert!(is_one_role_per_organization_violation(Some(
        SqlErr::UniqueConstraintViolation(
            "duplicate key value violates unique constraint \"user_roles_user_org_unique\""
                .to_string()
        )
    )));
}

#[test]
fn an_unrelated_unique_index_is_not_a_membership_conflict() {
    assert!(!is_one_role_per_organization_violation(Some(
        SqlErr::UniqueConstraintViolation("user_roles_user_global_role_unique".to_string())
    )));
    assert!(!is_one_role_per_organization_violation(Some(
        SqlErr::ForeignKeyConstraintViolation("user_roles_user_org_unique".to_string())
    )));
    assert!(!is_one_role_per_organization_violation(None));
}

/// A `DbErr` carrying no `SqlErr` must not be guessed at from its message text.
/// Only a parsed unique violation on the membership index may become a conflict.
#[tokio::test]
async fn create_leaves_an_unparsed_database_error_as_a_system_error() {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_errors([sea_orm::DbErr::Custom(
            "error returned from database: duplicate key value violates unique constraint \
             \"user_roles_user_global_role_unique\""
                .to_string(),
        )])
        .into_connection();

    let err = create(
        &db,
        Actor::new(Id::new_v4()),
        Id::new_v4(),
        Id::new_v4(),
        Role::Admin,
    )
    .await
    .expect_err("expected the unique violation to surface as an error");

    assert!(
        matches!(err.error_kind, EntityApiErrorKind::SystemError),
        "only the one-role-per-organization index may map to a conflict: {:?}",
        err.error_kind
    );
}

#[tokio::test]
async fn shares_administered_organization_is_true_for_an_admin_of_a_shared_org() -> Result<(), Error>
{
    let organization_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![column_row("organization_id", organization_id)]])
        .append_query_results([vec![column_row("id", Id::new_v4())]])
        .into_connection();

    assert!(shares_administered_organization(&db, Id::new_v4(), Id::new_v4()).await?);

    Ok(())
}

#[tokio::test]
async fn shares_administered_organization_is_false_for_a_mere_member() -> Result<(), Error> {
    // The requester holds `User` in the shared org, so the admin-scoped first query
    // finds nothing and the membership probe can only match on an empty id set.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<BTreeMap<String, Value>>::new()])
        .append_query_results([Vec::<BTreeMap<String, Value>>::new()])
        .into_connection();

    assert!(!shares_administered_organization(&db, Id::new_v4(), Id::new_v4()).await?);

    let log = db.into_transaction_log();
    assert_eq!(log.len(), 2, "expected exactly two queries");
    assert!(
        format!("{:?}", log[0]).contains("admin"),
        "first query must restrict the requester to Admin roles: {:?}",
        log[0]
    );
    assert!(
        format!("{:?}", log[1]).contains("Int(Some(1)), Int(Some(2))"),
        "membership probe must match nothing when the admin org set is empty: {:?}",
        log[1]
    );

    Ok(())
}

#[tokio::test]
async fn shares_administered_organization_is_false_without_overlap() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![column_row("organization_id", Id::new_v4())]])
        .append_query_results([Vec::<BTreeMap<String, Value>>::new()])
        .into_connection();

    assert!(!shares_administered_organization(&db, Id::new_v4(), Id::new_v4()).await?);

    Ok(())
}

#[tokio::test]
async fn count_organizations_for_user_counts_distinct_non_null_organizations() -> Result<(), Error>
{
    let user_id = Id::new_v4();
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![count_row(3)]])
        .into_connection();

    assert_eq!(count_organizations_for_user(&db, user_id).await?, 3);

    let log = db.into_transaction_log();
    let sql = format!("{:?}", log[0]);
    assert!(sql.contains("DISTINCT"), "count must be distinct: {sql}");
    assert!(
        sql.contains("IS NOT NULL"),
        "global roles must be excluded: {sql}"
    );

    Ok(())
}

#[test]
fn scope_roles_to_organization_keeps_only_the_target_org_and_global_roles() {
    let user_id = Id::new_v4();
    let target_organization_id = Id::new_v4();

    let in_scope = role_model(user_id, Some(target_organization_id), Role::Admin);
    let global = role_model(user_id, None, Role::SuperAdmin);
    let other_a = role_model(user_id, Some(Id::new_v4()), Role::User);
    let other_b = role_model(user_id, Some(Id::new_v4()), Role::Admin);

    let mut user = user_model(vec![
        in_scope.clone(),
        other_a.clone(),
        global.clone(),
        other_b.clone(),
    ]);

    scope_roles_to_organization(&mut user, target_organization_id);

    let retained_ids: Vec<Id> = user.roles.iter().map(|role| role.id).collect();
    assert_eq!(user.roles.len(), 2, "retained: {retained_ids:?}");
    assert_eq!(retained_ids, vec![in_scope.id, global.id]);
}

/// A global SuperAdmin grant has a null `organization_id`. Filtering on it would
/// render `organization_id = NULL`, which matches nothing, so the row would
/// survive while the audit claimed it was removed.
#[tokio::test]
async fn delete_removes_a_global_role_by_primary_key() -> Result<(), Error> {
    let global = role_model(Id::new_v4(), None, Role::SuperAdmin);
    let actor = Actor::new(Id::new_v4());

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results([sea_orm::MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .append_query_results([[entity::user_role_changes::Model {
            id: Id::new_v4(),
            actor_user_id: Some(actor.id()),
            target_user_id: global.user_id,
            organization_id: None,
            previous_role: Some(Role::SuperAdmin),
            new_role: None,
            changed_at: chrono::Utc::now().into(),
        }]])
        .into_connection();

    delete(&db, actor, &global).await?;

    let sql: Vec<String> = db
        .into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
        })
        .collect();

    let delete_sql = sql
        .iter()
        .find(|statement| statement.contains("DELETE"))
        .expect("the membership must be deleted");
    assert!(
        delete_sql.contains(r#""id" = $1"#),
        "must delete by primary key: {delete_sql}"
    );
    assert!(
        !delete_sql.contains("organization_id"),
        "a null organization_id must not reach the WHERE clause: {delete_sql}"
    );
    assert!(sql
        .iter()
        .any(|statement| statement.contains("user_role_changes")));

    Ok(())
}

/// If the row is already gone the delete is a no-op, so auditing it would record
/// a removal this caller did not perform.
#[tokio::test]
async fn delete_does_not_audit_when_no_row_was_removed() -> Result<(), Error> {
    let membership = role_model(Id::new_v4(), Some(Id::new_v4()), Role::Admin);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results([sea_orm::MockExecResult {
            last_insert_id: 0,
            rows_affected: 0,
        }])
        .into_connection();

    delete(&db, Actor::new(Id::new_v4()), &membership).await?;

    let sql: Vec<String> = db
        .into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
        })
        .collect();
    assert!(
        !sql.iter()
            .any(|statement| statement.contains("user_role_changes")),
        "nothing was deleted, so nothing may be audited: {sql:?}"
    );

    Ok(())
}
