use super::{AttachRoleParams, CreateMemberParams};
use domain::Id;

/// A misspelled key must not be silently dropped, leaving the caller to think
/// they set something they did not. Pins `deny_unknown_fields` on the body.
#[test]
fn attach_role_params_rejects_an_unknown_field() {
    let coach_id = Id::new_v4();
    let body = format!(r#"{{"role": "User", "coach": "{coach_id}"}}"#);

    let error = serde_json::from_str::<AttachRoleParams>(&body)
        .expect_err("a misspelled coach_id must be refused, not ignored");

    assert!(
        error.to_string().contains("unknown field"),
        "the caller must be told which key was rejected: {error}"
    );
}

/// `flatten` puts `coach_id` beside the user fields rather than nesting it, and
/// leaves a body predating the field still valid. Nesting either would break
/// deployed clients.
#[test]
fn create_member_params_reads_coach_id_beside_the_flattened_user_fields() {
    let coach_id = Id::new_v4();
    let user_fields = r#"
        "first_name": "Ada",
        "last_name": "Lovelace",
        "display_name": "Ada",
        "email": "ada@example.com",
        "timezone": "UTC"
    "#;

    let without_coach: CreateMemberParams = serde_json::from_str(&format!("{{{user_fields}}}"))
        .expect("a body predating coach_id must still deserialize");
    assert_eq!(without_coach.user.email, "ada@example.com");
    assert!(without_coach.coach_id.is_none());

    let with_coach: CreateMemberParams =
        serde_json::from_str(&format!(r#"{{{user_fields}, "coach_id": "{coach_id}"}}"#))
            .expect("coach_id must be read at the same level as the user fields");
    assert_eq!(with_coach.coach_id, Some(coach_id));
    assert_eq!(with_coach.user.email, "ada@example.com");
}
