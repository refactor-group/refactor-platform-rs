use super::*;
use chrono::Utc;
use sea_orm::{DatabaseBackend, MockDatabase, Value};
use std::collections::BTreeMap;

/// A list row whose coach is a member and whose coachee is not.
fn listed_row() -> BTreeMap<String, Value> {
    let now: DateTimeWithTimeZone = Utc::now().into();
    let name = |n: &str| Value::from(n.to_string());
    BTreeMap::from([
        ("id".to_string(), Id::new_v4().into()),
        ("coach_id".to_string(), Id::new_v4().into()),
        ("coachee_id".to_string(), Id::new_v4().into()),
        ("coach_first_name".to_string(), name("Coach")),
        ("coach_last_name".to_string(), name("One")),
        ("coachee_first_name".to_string(), name("Coachee")),
        ("coachee_last_name".to_string(), name("Two")),
        ("created_at".to_string(), now.into()),
        ("updated_at".to_string(), now.into()),
        ("coach_is_member".to_string(), true.into()),
        ("coachee_is_member".to_string(), false.into()),
    ])
}

/// The `EXISTS (...)` selected as `alias`.
pub(crate) fn membership_clause<'a>(sql: &'a str, alias: &str) -> &'a str {
    let end = sql
        .find(&format!(r#" AS "{alias}""#))
        .unwrap_or_else(|| panic!("no {alias} column: {sql}"));
    let start = sql[..end]
        .rfind("EXISTS(")
        .unwrap_or_else(|| panic!("{alias} is not an EXISTS: {sql}"));
    &sql[start..end]
}

#[tokio::test]
async fn m4_06_a_list_of_n_relationships_is_one_statement() -> Result<(), Error> {
    let rows: Vec<_> = (0..3).map(|_| listed_row()).collect();
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([rows])
        .into_connection();

    let listed = find_by_organization_with_user_names(&db, Id::new_v4()).await?;

    assert_eq!(listed.len(), 3);
    assert!(listed
        .iter()
        .all(|r| r.membership.coach_is_member && !r.membership.coachee_is_member));
    let log = db.into_transaction_log();
    assert_eq!(log.len(), 1, "{log:?}");
    let sql = &log[0].statements()[0].sql;
    for alias in ["coach_is_member", "coachee_is_member"] {
        let clause = membership_clause(sql, alias);
        assert!(
            clause.contains(r#"FROM "refactor_platform"."user_roles""#),
            "{alias}: {sql}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_empty_list_is_one_statement() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<BTreeMap<String, Value>>::new()])
        .into_connection();

    let listed = find_by_organization_with_user_names(&db, Id::new_v4()).await?;

    assert!(listed.is_empty());
    assert_eq!(db.into_transaction_log().len(), 1);
    Ok(())
}
