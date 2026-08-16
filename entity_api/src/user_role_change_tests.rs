use super::*;
use chrono::Utc;
use entity::user_role_changes::Model;
use sea_orm::{DatabaseBackend, MockDatabase, Statement, Value};

fn change_model(
    actor_user_id: Option<Id>,
    target_user_id: Id,
    organization_id: Option<Id>,
    previous_role: Option<Role>,
    new_role: Option<Role>,
) -> Model {
    Model {
        id: Id::new_v4(),
        actor_user_id,
        target_user_id,
        organization_id,
        previous_role,
        new_role,
        changed_at: Utc::now().into(),
    }
}

/// The single statement the mock saw, panicking if there was not exactly one.
fn only_statement(db: sea_orm::DatabaseConnection) -> Statement {
    let log = db.into_transaction_log();
    assert_eq!(log.len(), 1, "expected exactly one transaction");
    let statements = log[0].statements();
    assert_eq!(statements.len(), 1, "expected exactly one statement");
    statements[0].clone()
}

fn bound_values(statement: &Statement) -> Vec<Value> {
    statement
        .values
        .as_ref()
        .map(|values| values.0.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn record_inserts_a_grant_row() {
    let actor = Id::new_v4();
    let target = Id::new_v4();
    let organization = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[change_model(
            Some(actor),
            target,
            Some(organization),
            None,
            Some(Role::Admin),
        )]])
        .into_connection();

    record(
        &db,
        Some(Actor::new(actor)),
        target,
        Some(organization),
        None,
        Some(Role::Admin),
    )
    .await
    .expect("record should insert");

    let statement = only_statement(db);
    assert!(
        statement
            .sql
            .contains(r#"INSERT INTO "refactor_platform"."user_role_changes""#),
        "unexpected statement: {}",
        statement.sql
    );

    let values = bound_values(&statement);
    assert!(values.contains(&Value::Uuid(Some(Box::new(actor)))));
    assert!(values.contains(&Value::Uuid(Some(Box::new(target)))));
    assert!(values.contains(&Value::Uuid(Some(Box::new(organization)))));
    // A first grant has no previous role, and the new role binds as the enum's
    // string value rather than as free text.
    assert!(values.contains(&Value::String(None)));
    assert!(values.contains(&Value::String(Some(Box::new("admin".to_string())))));
}

#[tokio::test]
async fn record_inserts_a_removal_row_with_no_new_role() {
    let actor = Id::new_v4();
    let target = Id::new_v4();
    let organization = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[change_model(
            Some(actor),
            target,
            Some(organization),
            Some(Role::User),
            None,
        )]])
        .into_connection();

    record(
        &db,
        Some(Actor::new(actor)),
        target,
        Some(organization),
        Some(Role::User),
        None,
    )
    .await
    .expect("record should insert");

    let values = bound_values(&only_statement(db));
    assert!(values.contains(&Value::String(Some(Box::new("user".to_string())))));
    assert!(values.contains(&Value::String(None)));
    // The both-null row is rejected by a database CHECK constraint, which the mock
    // backend cannot enforce. Covered by manual verification instead.
}

#[tokio::test]
async fn record_accepts_a_null_actor() {
    let target = Id::new_v4();
    let organization = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[change_model(
            None,
            target,
            Some(organization),
            None,
            Some(Role::User),
        )]])
        .into_connection();

    record(
        &db,
        None,
        target,
        Some(organization),
        None,
        Some(Role::User),
    )
    .await
    .expect("record should insert");

    assert!(bound_values(&only_statement(db)).contains(&Value::Uuid(None)));
}
