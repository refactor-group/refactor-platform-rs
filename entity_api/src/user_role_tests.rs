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
        role: Role::User,
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

    let result = create(&db, Id::new_v4(), Id::new_v4(), Role::SuperAdmin).await;

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

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[inserted.clone()]])
        .into_connection();

    let created = create(&db, user_id, organization_id, Role::Admin).await?;

    assert_eq!(created.user_id, user_id);
    assert_eq!(created.organization_id, Some(organization_id));
    assert_eq!(created.role, Role::Admin);

    Ok(())
}

/// The mapping is matched on the constraint name rather than the message text,
/// which is locale and version sensitive. A `DbErr` carrying a real `SqlErr`
/// cannot be constructed by hand, so the policy is tested at its own boundary.
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

#[tokio::test]
async fn create_leaves_an_unrelated_unique_violation_as_a_system_error() {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_errors([sea_orm::DbErr::Custom(
            "error returned from database: duplicate key value violates unique constraint \
             \"user_roles_user_global_role_unique\""
                .to_string(),
        )])
        .into_connection();

    let err = create(&db, Id::new_v4(), Id::new_v4(), Role::Admin)
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
