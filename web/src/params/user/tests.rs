use super::{AttachRoleParams, CreateMemberParams};
use domain::users::Role;
use domain::Id;

/// The pre-coach_id payload must still deserialize, or existing clients break.
#[test]
fn create_member_params_accepts_a_body_with_only_user_fields() {
    let params: CreateMemberParams = serde_json::from_str(
        r#"{
            "first_name": "Ada",
            "last_name": "Lovelace",
            "display_name": "Ada",
            "email": "ada@example.com",
            "timezone": "UTC"
        }"#,
    )
    .expect("a body without coach_id must still deserialize");

    assert_eq!(params.user.email, "ada@example.com");
    assert_eq!(params.user.timezone, "UTC");
    assert!(params.coach_id.is_none());
}

#[test]
fn create_member_params_reads_the_coach_alongside_the_user_fields() {
    let coach_id = Id::new_v4();
    let body = format!(
        r#"{{
            "first_name": "Ada",
            "last_name": "Lovelace",
            "display_name": "Ada",
            "email": "ada@example.com",
            "timezone": "UTC",
            "coach_id": "{coach_id}"
        }}"#
    );

    let params: CreateMemberParams =
        serde_json::from_str(&body).expect("a body with coach_id must deserialize");

    assert_eq!(params.coach_id, Some(coach_id));
    assert_eq!(params.user.first_name, "Ada");
}

#[test]
fn attach_role_params_treats_coach_id_as_optional() {
    let params: AttachRoleParams =
        serde_json::from_str(r#"{"role": "User"}"#).expect("role-only body must deserialize");

    assert_eq!(params.role, Role::User);
    assert!(params.coach_id.is_none());
}

#[test]
fn attach_role_params_reads_the_coach() {
    let coach_id = Id::new_v4();
    let params: AttachRoleParams =
        serde_json::from_str(&format!(r#"{{"role": "Admin", "coach_id": "{coach_id}"}}"#))
            .expect("role and coach_id must deserialize");

    assert_eq!(params.role, Role::Admin);
    assert_eq!(params.coach_id, Some(coach_id));
}
