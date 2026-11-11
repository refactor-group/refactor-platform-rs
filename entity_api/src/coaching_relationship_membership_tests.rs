use super::*;
use chrono::Utc;
use sea_orm::{DatabaseBackend, MockDatabase, Value};
use std::collections::BTreeMap;

/// A list row for a relationship in `organization_id`.
fn listed_row(organization_id: Id) -> BTreeMap<String, Value> {
    let now: DateTimeWithTimeZone = Utc::now().into();
    let name = |n: &str| Value::from(n.to_string());
    BTreeMap::from([
        ("id".to_string(), Id::new_v4().into()),
        ("organization_id".to_string(), organization_id.into()),
        ("coach_id".to_string(), Id::new_v4().into()),
        ("coachee_id".to_string(), Id::new_v4().into()),
        ("coach_first_name".to_string(), name("Coach")),
        ("coach_last_name".to_string(), name("One")),
        ("coachee_first_name".to_string(), name("Coachee")),
        ("coachee_last_name".to_string(), name("Two")),
        ("created_at".to_string(), now.into()),
        ("updated_at".to_string(), now.into()),
    ])
}

/// A role row for `user_id` in `organization_id` (`None`: a global role).
fn role_row(user_id: Id, organization_id: Option<Id>) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("user_id".to_string(), user_id.into()),
        ("organization_id".to_string(), organization_id.into()),
    ])
}

fn id_of(row: &BTreeMap<String, Value>, column: &str) -> Id {
    match &row[column] {
        Value::Uuid(Some(id)) => *id,
        other => panic!("{column} is not a uuid: {other:?}"),
    }
}

#[tokio::test]
async fn m4_06_a_list_of_n_relationships_runs_one_membership_query() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let rows: Vec<_> = (0..3).map(|_| listed_row(organization_id)).collect();
    let members: Vec<_> = rows
        .iter()
        .map(|row| role_row(id_of(row, "coach_id"), Some(organization_id)))
        .collect();
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([rows.clone()])
        .append_query_results([members])
        .into_connection();

    let listed = find_by_organization_with_user_names(&db, organization_id).await?;

    assert_eq!(listed.len(), 3);
    assert!(listed
        .iter()
        .all(|r| r.coach_is_member && !r.coachee_is_member));
    let log = db.into_transaction_log();
    let membership_queries = log
        .iter()
        .filter(|t| format!("{t:?}").contains("user_roles"))
        .count();
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(membership_queries, 1, "{log:?}");
    Ok(())
}

#[tokio::test]
async fn a_role_counts_only_in_its_own_organization() -> Result<(), Error> {
    let (user, o1, o2) = (Id::new_v4(), Id::new_v4(), Id::new_v4());
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![role_row(user, Some(o1))]])
        .into_connection();

    let members =
        crate::user_role::organization_memberships(&db, &[(user, o1), (user, o2)]).await?;

    assert!(members.contains(&(user, o1)));
    assert!(!members.contains(&(user, o2)));
    Ok(())
}

#[tokio::test]
async fn a_global_super_admin_is_a_member_of_every_organization() -> Result<(), Error> {
    let (user, o1, o2) = (Id::new_v4(), Id::new_v4(), Id::new_v4());
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![role_row(user, None)]])
        .into_connection();

    let members =
        crate::user_role::organization_memberships(&db, &[(user, o1), (user, o2)]).await?;

    assert_eq!(members.len(), 2, "{members:?}");
    Ok(())
}

#[tokio::test]
async fn an_empty_list_runs_no_membership_query() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<BTreeMap<String, Value>>::new()])
        .into_connection();

    let listed = find_by_organization_with_user_names(&db, Id::new_v4()).await?;

    assert!(listed.is_empty());
    assert_eq!(db.into_transaction_log().len(), 1);
    Ok(())
}
