use super::*;
use crate::roles::Role;
use crate::user_roles;
use crate::users;

fn create_test_relationship(coach_id: Id, coachee_id: Id) -> Model {
    relationship_of(Kind::Coaching, coach_id, coachee_id)
}

fn relationship_of(kind: Kind, coach_id: Id, coachee_id: Id) -> Model {
    let now = chrono::Utc::now().fixed_offset();
    Model {
        id: Id::new_v4(),
        organization_id: Id::new_v4(),
        coach_id,
        coachee_id,
        kind,
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
        working_hours_start: crate::working_hours::default_start(),
        working_hours_end: crate::working_hours::default_end(),
        roles,
        invite_status: None,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn admits_a_coach_who_is_still_a_member() {
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

    assert!(relationship.is_participant_and_member(&user));
}

#[test]
fn admits_a_coachee_who_is_still_a_member() {
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

    assert!(relationship.is_participant_and_member(&user));
}

#[test]
fn denies_a_coach_removed_from_the_organization() {
    let coach_id = Id::new_v4();
    let relationship = create_test_relationship(coach_id, Id::new_v4());
    let user = create_test_user(coach_id, vec![]);

    assert!(!relationship.is_participant_and_member(&user));
}

#[test]
fn denies_a_coachee_removed_from_the_organization() {
    let coachee_id = Id::new_v4();
    let relationship = create_test_relationship(Id::new_v4(), coachee_id);
    let user = create_test_user(coachee_id, vec![]);

    assert!(!relationship.is_participant_and_member(&user));
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

    assert!(!relationship.is_participant_and_member(&user));
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

    assert!(!relationship.is_participant_and_member(&user));
}

#[test]
fn admits_a_global_super_admin_participant() {
    let coach_id = Id::new_v4();
    let relationship = create_test_relationship(coach_id, Id::new_v4());
    let user = create_test_user(
        coach_id,
        vec![create_test_role(coach_id, Role::SuperAdmin, None)],
    );

    assert!(relationship.is_participant_and_member(&user));
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

    assert!(!relationship.is_participant_and_member(&user));
}

#[test]
fn admits_a_member_holding_roles_in_several_organizations() {
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

    assert!(relationship.is_participant_and_member(&user));
}

/// A user holding a plain role in `relationship`'s organization.
fn member_of(relationship: &Model, user_id: Id) -> users::Model {
    create_test_user(
        user_id,
        vec![create_test_role(
            user_id,
            Role::User,
            Some(relationship.organization_id),
        )],
    )
}

#[test]
fn only_a_coaching_relationships_member_coach_gets_a_scheduling_permit() {
    let (coach_id, coachee_id) = (Id::new_v4(), Id::new_v4());
    let relationship = create_test_relationship(coach_id, coachee_id);

    let permit =
        SchedulingPermit::verify(relationship.clone(), &member_of(&relationship, coach_id))
            .expect("the member coach may schedule sessions");
    assert_eq!(permit.user_id(), coach_id);
    assert_eq!(permit.relationship().id, relationship.id);

    for (who, user) in [
        ("coachee", member_of(&relationship, coachee_id)),
        ("coach who left", create_test_user(coach_id, vec![])),
        ("stranger", member_of(&relationship, Id::new_v4())),
    ] {
        assert!(
            SchedulingPermit::verify(relationship.clone(), &user).is_none(),
            "{who}"
        );
    }
}

#[test]
fn a_global_super_admin_coach_gets_a_scheduling_permit_without_an_org_role() {
    let coach_id = Id::new_v4();
    let relationship = create_test_relationship(coach_id, Id::new_v4());
    let user = create_test_user(
        coach_id,
        vec![create_test_role(coach_id, Role::SuperAdmin, None)],
    );

    assert!(SchedulingPermit::verify(relationship, &user).is_some());
}

#[test]
fn both_member_colleagues_get_a_scheduling_permit_and_one_who_left_does_not() {
    let (first, second) = (Id::new_v4(), Id::new_v4());
    let relationship = relationship_of(Kind::Colleague, first, second);

    for user_id in [first, second] {
        let permit =
            SchedulingPermit::verify(relationship.clone(), &member_of(&relationship, user_id))
                .expect("a member colleague may schedule sessions");
        assert_eq!(permit.user_id(), user_id);
    }
    assert!(SchedulingPermit::verify(relationship, &create_test_user(second, vec![])).is_none());
}
