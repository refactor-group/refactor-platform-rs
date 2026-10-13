use super::*;
use sea_orm::{DatabaseBackend, MockDatabase, Value};
use std::collections::BTreeMap;

fn super_admin_visibility() -> Visibility {
    Visibility {
        user_id: Id::new_v4(),
        is_super_admin: true,
        admin_org_ids: vec![],
        member_org_ids: vec![],
    }
}

#[tokio::test]
async fn an_org_admins_visibility_is_participant_only() {
    // The Admin role adds no relationship visibility: the query carries the
    // participant predicates and exactly one organization_id membership test —
    // no admin-org tier (search mirrors grants_access_to).
    let org_id = Id::new_v4();
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![Vec::<coaching_relationships::Model>::new()])
        .into_connection();
    let visibility = Visibility {
        user_id: Id::new_v4(),
        is_super_admin: false,
        admin_org_ids: vec![org_id],
        member_org_ids: vec![org_id],
    };

    let resolved = resolve_visible_relationship_ids(&db, &visibility, None, None)
        .await
        .expect("resolves");

    assert_eq!(resolved, Some(vec![]));
    let log = db.into_transaction_log();
    let sql = format!("{:?}", log[0]);
    assert!(sql.contains(r#"\"coach_id\" ="#));
    assert!(sql.contains(r#"\"coachee_id\" ="#));
    assert_eq!(
        sql.matches(r#"\"organization_id\" IN"#).count(),
        1,
        "one membership predicate, no admin-org tier: {sql}"
    );
}

#[tokio::test]
async fn super_admin_relationship_filter_alone_needs_no_query() {
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
    let rel_id = Id::new_v4();

    let resolved =
        resolve_visible_relationship_ids(&db, &super_admin_visibility(), None, Some(rel_id))
            .await
            .expect("resolves");

    assert_eq!(resolved, Some(vec![rel_id]));
    assert!(db.into_transaction_log().is_empty());
}

#[tokio::test]
async fn super_admin_org_and_relationship_filters_intersect() {
    let rel_id = Id::new_v4();
    let org_id = Id::new_v4();
    // The query selects bare ids, so the mock row carries only `id`.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![BTreeMap::from([("id", Value::from(rel_id))])]])
        .into_connection();

    let resolved = resolve_visible_relationship_ids(
        &db,
        &super_admin_visibility(),
        Some(org_id),
        Some(rel_id),
    )
    .await
    .expect("resolves");

    assert_eq!(resolved, Some(vec![rel_id]));
    let log = db.into_transaction_log();
    let sql = format!("{:?}", log[0]);
    assert!(sql.contains(r#"\"coaching_relationships\".\"id\" ="#));
    assert!(sql.contains(r#"\"coaching_relationships\".\"organization_id\" ="#));
}

#[tokio::test]
async fn super_admin_relationship_outside_the_requested_org_resolves_to_nothing() {
    // The DB finds no relationship matching BOTH filters.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![Vec::<coaching_relationships::Model>::new()])
        .into_connection();

    let resolved = resolve_visible_relationship_ids(
        &db,
        &super_admin_visibility(),
        Some(Id::new_v4()),
        Some(Id::new_v4()),
    )
    .await
    .expect("resolves");

    assert_eq!(resolved, Some(vec![]));
}
