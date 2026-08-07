use super::*;
use mockito::{Server, ServerGuard};

async fn setup_test_server() -> ServerGuard {
    Server::new_async().await
}

fn test_user(first_name: &str, last_name: &str, email: &str) -> users::Model {
    users::Model {
        id: Id::new_v4(),
        first_name: first_name.to_string(),
        last_name: last_name.to_string(),
        email: email.to_string(),
        display_name: Some(format!("{first_name} {last_name}")),
        password: Some("hashed_password".to_string()),
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        role: Role::User,
        roles: vec![],
        invite_status: None,
        created_at: chrono::Utc::now().fixed_offset(),
        updated_at: chrono::Utc::now().fixed_offset(),
    }
}

fn test_organization() -> organizations::Model {
    organizations::Model {
        id: Id::new_v4(),
        name: "Acme Corp".to_string(),
        logo: None,
        slug: "acme-corp".to_string(),
        created_at: chrono::Utc::now().fixed_offset(),
        updated_at: chrono::Utc::now().fixed_offset(),
        archived_at: None,
        archived_by: None,
    }
}

#[test]
fn role_display_name_maps_admin_and_user() {
    assert_eq!(role_display_name(Role::Admin), "Admin");
    assert_eq!(role_display_name(Role::User), "Member");
    // Unreachable from the route, but must not panic.
    assert_eq!(role_display_name(Role::SuperAdmin), "Member");
}

#[test]
fn url_path_falls_back_to_default_when_empty() {
    let config = Config::from_args(["test", "--added-to-organization-email-url-path="]);
    assert_eq!(config.added_to_organization_email_url_path(), "/dashboard");

    let config = Config::from_args(["test"]);
    assert_eq!(config.added_to_organization_email_url_path(), "/dashboard");

    let config = Config::from_args([
        "test",
        "--added-to-organization-email-url-path=/organizations/{organization_id}",
    ]);
    assert_eq!(
        config.added_to_organization_email_url_path(),
        "/organizations/{organization_id}"
    );
}

#[tokio::test]
async fn notify_without_template_id_logs_and_returns() {
    let mut server = setup_test_server().await;
    let config = Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
    ]);
    assert!(
        config.added_to_organization_email_template_id().is_none(),
        "template ID should be unset for this test"
    );

    // No template ID means no send attempt at all.
    let _mock = server
        .mock("POST", "/emails")
        .expect(0)
        .create_async()
        .await;

    // Returns `()`: reaching this line at all is the assertion.
    notify_added_to_organization(
        &config,
        &test_user("John", "Doe", "john@example.com"),
        &test_user("Sarah", "Coach", "sarah@example.com"),
        &test_organization(),
        Role::User,
    )
    .await;
}

#[tokio::test]
async fn send_added_to_organization_email_wire_contract() {
    let mut server = setup_test_server().await;
    let config = Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        "--added-to-organization-email-template-id=added_template_123",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
    ]);

    let user = test_user("John", "Doe", "john@example.com");
    let inviter = test_user("Sarah", "Coach", "sarah@example.com");
    let organization = test_organization();

    let _mock = server
        .mock("POST", "/emails")
        .match_header("authorization", "Bearer test_api_key_123")
        .match_body(mockito::Matcher::Json(serde_json::json!({
            "from": FROM_ADDRESS,
            "to": ["\"John Doe\" <john@example.com>"],
            "template": {
                "id": "added_template_123",
                "variables": {
                    "first_name": "John",
                    "last_name": "Doe",
                    "organization_name": "Acme Corp",
                    "role_name": "Admin",
                    "inviter_first_name": "Sarah",
                    "inviter_full_name": "Sarah Coach",
                    "organization_url": "https://app.example.com/dashboard",
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_added_to_org"}"#)
        .expect(1)
        .create_async()
        .await;

    let result =
        send_added_to_organization_email(&config, &user, &inviter, &organization, Role::Admin)
            .await;
    assert!(result.is_ok(), "send failed: {result:?}");
}

#[tokio::test]
async fn organization_id_placeholder_is_substituted() {
    let mut server = setup_test_server().await;
    let config = Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        "--added-to-organization-email-template-id=added_template_123",
        "--added-to-organization-email-url-path=/organizations/{organization_id}",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
    ]);

    let organization = test_organization();
    let expected_url = format!("https://app.example.com/organizations/{}", organization.id);

    let _mock = server
        .mock("POST", "/emails")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "template": {
                "variables": {
                    "role_name": "Member",
                    "organization_url": expected_url,
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_added_to_org"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_added_to_organization_email(
        &config,
        &test_user("John", "Doe", "john@example.com"),
        &test_user("Sarah", "Coach", "sarah@example.com"),
        &organization,
        Role::User,
    )
    .await;
    assert!(result.is_ok(), "send failed: {result:?}");
}
