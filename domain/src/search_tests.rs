use super::*;
use crate::user_roles;
use chrono::Utc;

fn user_with_roles(roles: Vec<user_roles::Model>) -> users::Model {
    users::Model {
        id: Id::new_v4(),
        email: "search-scope@test.dev".to_string(),
        first_name: "Scope".to_string(),
        last_name: "Test".to_string(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        roles,
        invite_status: None,
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    }
}

fn role(role: Role, organization_id: Option<Id>) -> user_roles::Model {
    user_roles::Model {
        id: Id::new_v4(),
        role,
        organization_id,
        user_id: Id::new_v4(),
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    }
}

#[test]
fn scope_for_a_regular_member_has_membership_but_no_admin_orgs() {
    let org = Id::new_v4();
    let user = user_with_roles(vec![role(Role::User, Some(org))]);
    let scope = scope_for(&user);
    assert_eq!(scope.user_id, user.id);
    assert!(!scope.is_super_admin);
    assert!(scope.admin_org_ids.is_empty());
    assert_eq!(scope.member_org_ids, vec![org]);
}

#[test]
fn scope_for_an_org_admin_is_both_admin_and_member_of_that_org() {
    let admin_org = Id::new_v4();
    let member_org = Id::new_v4();
    let user = user_with_roles(vec![
        role(Role::Admin, Some(admin_org)),
        role(Role::User, Some(member_org)),
    ]);
    let scope = scope_for(&user);
    assert!(!scope.is_super_admin);
    assert_eq!(scope.admin_org_ids, vec![admin_org]);
    assert_eq!(scope.member_org_ids, vec![admin_org, member_org]);
}

#[cfg(feature = "mock")]
mod mock_tests {
    use super::*;
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

        let resolved =
            resolve_visible_relationship_ids(&db, &super_admin_scope(), None, Some(rel_id))
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
}

#[test]
fn scope_for_super_admin_requires_the_global_role_row() {
    let org = Id::new_v4();
    // An org-scoped SuperAdmin row is not the global role.
    let scoped = user_with_roles(vec![role(Role::SuperAdmin, Some(org))]);
    assert!(!scope_for(&scoped).is_super_admin);

    let global = user_with_roles(vec![role(Role::SuperAdmin, None)]);
    assert!(scope_for(&global).is_super_admin);
}
