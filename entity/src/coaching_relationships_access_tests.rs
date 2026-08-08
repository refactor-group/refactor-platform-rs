use super::*;
use crate::user_roles;
use crate::users;

fn create_test_relationship(coach_id: Id, coachee_id: Id) -> Model {
    let now = chrono::Utc::now().fixed_offset();
    Model {
        id: Id::new_v4(),
        organization_id: Id::new_v4(),
        coach_id,
        coachee_id,
        slug: "test-slug".to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn create_test_role(user_id: Id, role: Role, organization_id: Option<Id>) -> user_roles::Model {
    let now = chrono::Utc::now().fixed_offset();
    user_roles::Model {
        id: Id::new_v4(),
        role,
        organization_id,
        user_id,
        created_at: now,
        updated_at: now,
    }
}

fn create_test_user(id: Id, roles: Vec<user_roles::Model>) -> users::Model {
    let now = chrono::Utc::now().fixed_offset();
    users::Model {
        id,
        email: "test@example.com".to_string(),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        role: Role::default(),
        roles,
        invite_status: None,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn grants_access_to_a_coach_who_is_still_a_member() {
    let coach_id = Id::new_v4();
    let relationship = create_test_relationship(coach_id, Id::new_v4());
    let user = create_test_user(
        coach_id,
        vec![create_test_role(
            coach_id,
            Role::User,
            Some(relationship.organization_id),
        )],
    );

    assert!(relationship.grants_access_to(&user));
}

#[test]
fn grants_access_to_a_coachee_who_is_still_a_member() {
    let coachee_id = Id::new_v4();
    let relationship = create_test_relationship(Id::new_v4(), coachee_id);
    let user = create_test_user(
        coachee_id,
        vec![create_test_role(
            coachee_id,
            Role::User,
            Some(relationship.organization_id),
        )],
    );

    assert!(relationship.grants_access_to(&user));
}

#[test]
fn denies_a_coach_removed_from_the_organization() {
    let coach_id = Id::new_v4();
    let relationship = create_test_relationship(coach_id, Id::new_v4());
    let user = create_test_user(coach_id, vec![]);

    assert!(!relationship.grants_access_to(&user));
}

#[test]
fn denies_a_coachee_removed_from_the_organization() {
    let coachee_id = Id::new_v4();
    let relationship = create_test_relationship(Id::new_v4(), coachee_id);
    let user = create_test_user(coachee_id, vec![]);

    assert!(!relationship.grants_access_to(&user));
}

/// Membership is checked against this relationship's organization, not just any.
#[test]
fn denies_a_participant_whose_only_role_is_in_another_organization() {
    let coachee_id = Id::new_v4();
    let relationship = create_test_relationship(Id::new_v4(), coachee_id);
    let user = create_test_user(
        coachee_id,
        vec![create_test_role(coachee_id, Role::User, Some(Id::new_v4()))],
    );

    assert!(!relationship.grants_access_to(&user));
}

/// Membership alone never substitutes for participation.
#[test]
fn denies_a_non_participant_who_is_a_member_of_the_organization() {
    let relationship = create_test_relationship(Id::new_v4(), Id::new_v4());
    let outsider_id = Id::new_v4();
    let user = create_test_user(
        outsider_id,
        vec![create_test_role(
            outsider_id,
            Role::User,
            Some(relationship.organization_id),
        )],
    );

    assert!(!relationship.grants_access_to(&user));
}

#[test]
fn grants_access_to_a_global_super_admin_participant() {
    let coach_id = Id::new_v4();
    let relationship = create_test_relationship(coach_id, Id::new_v4());
    let user = create_test_user(
        coach_id,
        vec![create_test_role(coach_id, Role::SuperAdmin, None)],
    );

    assert!(relationship.grants_access_to(&user));
}

/// A global SuperAdmin still gets nothing without participation.
#[test]
fn denies_a_global_super_admin_who_is_not_a_participant() {
    let relationship = create_test_relationship(Id::new_v4(), Id::new_v4());
    let outsider_id = Id::new_v4();
    let user = create_test_user(
        outsider_id,
        vec![create_test_role(outsider_id, Role::SuperAdmin, None)],
    );

    assert!(!relationship.grants_access_to(&user));
}

#[test]
fn grants_access_to_a_member_holding_roles_in_several_organizations() {
    let coachee_id = Id::new_v4();
    let relationship = create_test_relationship(Id::new_v4(), coachee_id);
    let user = create_test_user(
        coachee_id,
        vec![
            create_test_role(coachee_id, Role::User, Some(Id::new_v4())),
            create_test_role(coachee_id, Role::Admin, Some(relationship.organization_id)),
            create_test_role(coachee_id, Role::User, Some(Id::new_v4())),
        ],
    );

    assert!(relationship.grants_access_to(&user));
}
