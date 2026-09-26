use super::*;
use chrono::Utc;
use sea_orm::{DatabaseBackend, MockDatabase};

fn super_admin_scope() -> Scope {
    Scope {
        user_id: Id::new_v4(),
        is_super_admin: true,
        admin_org_ids: vec![],
        member_org_ids: vec![],
    }
}

fn relationship(id: Id, organization_id: Id) -> coaching_relationships::Model {
    coaching_relationships::Model {
        id,
        organization_id,
        coach_id: Id::new_v4(),
        coachee_id: Id::new_v4(),
        slug: "coach-coachee".to_string(),
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    }
}

#[tokio::test]
async fn super_admin_relationship_filter_alone_needs_no_query() {
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
    let rel_id = Id::new_v4();

    let resolved = resolve_visible_relationship_ids(&db, &super_admin_scope(), None, Some(rel_id))
        .await
        .expect("resolves");

    assert_eq!(resolved, Some(vec![rel_id]));
    assert!(db.into_transaction_log().is_empty());
}

#[tokio::test]
async fn super_admin_org_and_relationship_filters_intersect() {
    let rel_id = Id::new_v4();
    let org_id = Id::new_v4();
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![relationship(rel_id, org_id)]])
        .into_connection();

    let resolved =
        resolve_visible_relationship_ids(&db, &super_admin_scope(), Some(org_id), Some(rel_id))
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
        &super_admin_scope(),
        Some(Id::new_v4()),
        Some(Id::new_v4()),
    )
    .await
    .expect("resolves");

    assert_eq!(resolved, Some(vec![]));
}
