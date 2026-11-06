use super::{AttachRoleParams, CreateMemberParams, UpdateParams, UpdateRoleParams, UpdateUser};
use crate::Error;
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::NaiveTime;
use domain::error::{DomainErrorKind, Error as DomainError};
use domain::user::WorkingHoursPatch;
use domain::users::Role;
use domain::{Id, IntoUpdateMap};
use sea_orm::Value;

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

/// This endpoint changes a role and nothing else. Without `deny_unknown_fields` a
/// client sending `coach_id` gets a 200 and can reasonably believe a coach was
/// assigned, which is the failure `create` already has.
#[test]
fn update_role_params_rejects_an_unknown_field() {
    let coach_id = Id::new_v4();
    let body = format!(r#"{{"role": "Admin", "coach_id": "{coach_id}"}}"#);

    let error = serde_json::from_str::<UpdateRoleParams>(&body)
        .expect_err("coach_id must be refused, not ignored");

    assert!(
        error.to_string().contains("unknown field"),
        "the caller must be told which key was rejected: {error}"
    );
}

/// The role is PascalCase on the wire and lowercase in Postgres. Reading the
/// column or a migration suggests lowercase, so the wrong case is easy to send and
/// the resulting 422 is easy to misread as a logic bug.
#[test]
fn update_role_params_reads_the_pascal_case_wire_form() {
    let params: UpdateRoleParams =
        serde_json::from_str(r#"{"role": "Admin"}"#).expect("PascalCase is the wire form");
    assert_eq!(params.role, Role::Admin);

    serde_json::from_str::<UpdateRoleParams>(r#"{"role": "admin"}"#)
        .expect_err("the lowercase database form is not the wire form");
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

fn update_params(start: Option<&str>, end: Option<&str>) -> UpdateParams {
    UpdateParams {
        email: None,
        first_name: Some("Ada".to_string()),
        last_name: None,
        display_name: None,
        github_profile_url: None,
        timezone: None,
        default_coaching_session_duration_minutes: None,
        working_hours_start: start.map(str::to_string),
        working_hours_end: end.map(str::to_string),
    }
}

/// Raw strings must never reach a TIME column; only parsed times may.
#[test]
fn update_params_leave_working_hours_out_of_the_update_map() {
    let map = update_params(Some("09:00"), Some("17:00")).into_update_map();

    assert!(map.get_value("working_hours_start").is_none());
    assert!(map.get_value("working_hours_end").is_none());
    assert!(map.get_value("first_name").is_some());
}

#[test]
fn update_user_carries_only_the_parsed_working_hours_sent() {
    let start = NaiveTime::from_hms_opt(9, 30, 0).expect("valid time");
    let map = UpdateUser {
        params: update_params(None, None),
        working_hours: WorkingHoursPatch {
            start: Some(start),
            end: None,
        },
    }
    .into_update_map();

    assert_eq!(
        map.get_value("working_hours_start"),
        Some(&Value::ChronoTime(Some(start)))
    );
    assert!(map.get_value("working_hours_end").is_none());
}

#[tokio::test]
async fn a_working_hours_validation_error_renders_a_422_body() {
    let response = Error::from(DomainError {
        source: None,
        error_kind: DomainErrorKind::Validation(
            "working_hours_end must be HH:MM on a 30-minute step".to_string(),
        ),
    })
    .into_response();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body collects");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("body is JSON");
    assert_eq!(
        body,
        serde_json::json!({
            "status_code": 422,
            "error": "validation_error",
            "message": "working_hours_end must be HH:MM on a 30-minute step",
        })
    );
}
