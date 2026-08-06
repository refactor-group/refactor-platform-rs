use super::*;

fn pair(user_id: &str, organization_id: &str, role_count: i64) -> DuplicatePair {
    DuplicatePair {
        user_id: user_id.to_string(),
        organization_id: organization_id.to_string(),
        role_count,
    }
}

#[test]
fn no_error_when_every_pair_holds_one_role() {
    assert!(duplicate_pairs_error(&[]).is_none());
}

#[test]
fn error_names_every_offending_pair_and_its_role_count() {
    let pairs = vec![pair("user-a", "org-a", 2), pair("user-b", "org-b", 3)];

    let Some(DbErr::Custom(message)) = duplicate_pairs_error(&pairs) else {
        panic!("expected a Custom error for duplicate pairs");
    };

    assert!(
        message.contains("2 user/organization pair(s)"),
        "message must count the offenders: {message}"
    );
    assert!(
        message.contains("(user_id=user-a, organization_id=org-a, roles=2)"),
        "message must name the first offender: {message}"
    );
    assert!(
        message.contains("(user_id=user-b, organization_id=org-b, roles=3)"),
        "message must name the second offender: {message}"
    );
    assert!(
        message.contains("user_roles_user_org_unique"),
        "message must name the index it refused to create: {message}"
    );
}
