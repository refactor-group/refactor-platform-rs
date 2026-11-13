use super::*;
use sea_orm::{DatabaseBackend, MockDatabase};

fn super_admin_visibility() -> Visibility {
    Visibility {
        user_id: Id::new_v4(),
        is_super_admin: true,
        admin_org_ids: vec![],
        member_org_ids: vec![],
    }
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
