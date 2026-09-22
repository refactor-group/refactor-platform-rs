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

#[test]
fn scope_for_super_admin_requires_the_global_role_row() {
    let org = Id::new_v4();
    // An org-scoped SuperAdmin row is not the global role.
    let scoped = user_with_roles(vec![role(Role::SuperAdmin, Some(org))]);
    assert!(!scope_for(&scoped).is_super_admin);

    let global = user_with_roles(vec![role(Role::SuperAdmin, None)]);
    assert!(scope_for(&global).is_super_admin);
}
