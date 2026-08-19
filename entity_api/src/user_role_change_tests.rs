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

/// A single-column mock row, for the id-only administered-organization select.
fn column_row(column: &str, value: Id) -> std::collections::BTreeMap<String, Value> {
    std::collections::BTreeMap::from([(column.to_string(), value.into())])
}

/// Every statement the mock saw, in order.
fn logged_statements(db: sea_orm::DatabaseConnection) -> Vec<Statement> {
    db.into_transaction_log()
        .iter()
        .flat_map(|transaction| transaction.statements().to_vec())
        .collect()
}

#[tokio::test]
async fn was_member_reports_a_prior_change_in_an_administered_organization() -> Result<(), Error> {
    let organization_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![column_row("organization_id", organization_id)]])
        .append_query_results([vec![column_row("id", Id::new_v4())]])
        .into_connection();

    assert!(was_member_of_administered_organization(&db, Id::new_v4(), Id::new_v4()).await?);

    Ok(())
}

/// I-C2. The whole security argument for this reach is that the caller learns only
/// about their own organization's history. History elsewhere must stay invisible, or
/// the predicate quietly becomes the general exact-email match that was rejected.
#[tokio::test]
async fn was_member_ignores_history_outside_the_administered_organizations() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![column_row("organization_id", Id::new_v4())]])
        .append_query_results([Vec::<std::collections::BTreeMap<String, Value>>::new()])
        .into_connection();

    assert!(!was_member_of_administered_organization(&db, Id::new_v4(), Id::new_v4()).await?);

    Ok(())
}

/// A requester who administers nothing can match nothing, and the probe still runs.
#[tokio::test]
async fn was_member_is_false_for_a_requester_who_administers_nothing() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<std::collections::BTreeMap<String, Value>>::new()])
        .append_query_results([Vec::<std::collections::BTreeMap<String, Value>>::new()])
        .into_connection();

    assert!(!was_member_of_administered_organization(&db, Id::new_v4(), Id::new_v4()).await?);

    let statements = logged_statements(db);
    assert_eq!(
        statements.len(),
        2,
        "the probe must run even with no administered organizations: {statements:?}"
    );
    // The role is a bound parameter, not SQL text: an enum filter renders as
    // `CAST($n AS "role")`, so the column belongs in the SQL check and the value in
    // the values check.
    assert!(
        statements[0].sql.contains(r#""user_roles"."role""#),
        "the first query must filter on the role column: {}",
        statements[0].sql
    );
    assert!(
        bound_values(&statements[0]).contains(&Value::String(Some(Box::new("admin".to_string())))),
        "and must restrict the requester to Admin: {:?}",
        bound_values(&statements[0])
    );
    assert!(
        statements[1].sql.contains("user_role_changes"),
        "the second must probe the audit table: {}",
        statements[1].sql
    );

    Ok(())
}

/// I-C3, this half. The caller composes this with
/// `user_role::shares_administered_organization` and promises a constant query count
/// so response timing cannot be used to enumerate accounts. That only holds if this
/// predicate is unconditional: an early return on an empty organization set would
/// make the count depend on the answer.
#[tokio::test]
async fn was_member_always_issues_exactly_two_queries() -> Result<(), Error> {
    let organization_id = Id::new_v4();

    let hit = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![column_row("organization_id", organization_id)]])
        .append_query_results([vec![column_row("id", Id::new_v4())]])
        .into_connection();
    was_member_of_administered_organization(&hit, Id::new_v4(), Id::new_v4()).await?;

    let miss = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<std::collections::BTreeMap<String, Value>>::new()])
        .append_query_results([Vec::<std::collections::BTreeMap<String, Value>>::new()])
        .into_connection();
    was_member_of_administered_organization(&miss, Id::new_v4(), Id::new_v4()).await?;

    // Compared to each other, not to a literal: a future change that alters both
    // uniformly is fine, one that makes the count depend on the answer is not.
    assert_eq!(
        logged_statements(hit).len(),
        logged_statements(miss).len(),
        "query count must not depend on whether the target was ever a member"
    );

    Ok(())
}
