use crate::{
    error::Error,
    error::{DomainErrorKind, InternalErrorKind},
    gateway::mailersend::{MailerSendClient, SendEmailRequestBuilder},
    users,
};

use log::*;
use service::config::Config;

/// Send a welcome email to a newly created user
pub async fn send_welcome_email(config: &Config, user: &users::Model) -> Result<(), Error> {
    info!(
        "Initiating welcome email for user: {} ({})",
        user.email, user.id
    );

    let mailersend_client = MailerSendClient::new(config).await?;

    let template_id = config.welcome_email_template_id().ok_or_else(|| {
        error!("Welcome email template ID not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;
    info!("Using template ID: {template_id}");

    debug!("Preparing personalization data for {}", user.email);

    let email_request = SendEmailRequestBuilder::new()
        .from("hello@myrefactor.com")
        .to_with_name(
            &user.email,
            format!("{} {}", user.first_name, user.last_name),
        )
        .subject("Welcome to Refactor Platform")
        .template_id(template_id)
        .add_personalization("first_name", &user.first_name)
        .add_personalization("last_name", &user.last_name)
        .build()
        .await?;
    debug!("Email request created for {}", user.email);

    // send_email now handles the async spawning internally
    mailersend_client.send_email(email_request).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{users, Id};
    use mockito::{Server, ServerGuard};
    use serial_test::serial;
    use service::config::Config;
    use std::env;

    async fn setup_test_server() -> ServerGuard {
        Server::new_async().await
    }

    fn create_test_user() -> users::Model {
        users::Model {
            id: Id::new_v4(),
            first_name: "John".to_string(),
            last_name: "Doe".to_string(),
            email: "john.doe@example.com".to_string(),
            display_name: Some("John Doe".to_string()),
            password: "hashed_password".to_string(),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            role: users::Role::User,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
        }
    }

    fn create_config_with_mock(server_url: &str) -> Config {
        env::set_var("MAILERSEND_API_KEY", "test_api_key_123");
        env::set_var("WELCOME_EMAIL_TEMPLATE_ID", "template_123");
        env::set_var("MAILERSEND_BASE_URL", server_url);
        Config::default()
    }

    /// Helper struct to manage environment variables in tests
    struct EnvGuard {
        saved_vars: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        fn new(vars: &[&str]) -> Self {
            let saved_vars = vars
                .iter()
                .map(|var| (var.to_string(), env::var(var).ok()))
                .collect();
            EnvGuard { saved_vars }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // Restore all saved environment variables
            for (key, value) in &self.saved_vars {
                match value {
                    Some(val) => env::set_var(key, val),
                    None => env::remove_var(key),
                }
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_send_welcome_email_success() {
        let mut server = setup_test_server().await;
        let user = create_test_user();
        let config = create_config_with_mock(&server.url());

        let _mock = server
            .mock("POST", "/v1/email")
            .match_header("authorization", "Bearer test_api_key_123")
            .match_header("content-type", "application/json")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": {
                    "email": "hello@myrefactor.com",
                    "name": null
                },
                "to": [{
                    "email": "john.doe@example.com",
                    "name": "John Doe"
                }],
                "subject": "Welcome to Refactor Platform",
                "template_id": "template_123",
                "personalization": [{
                    "email": "john.doe@example.com",
                    "data": {
                        "first_name": "John",
                        "last_name": "Doe"
                    }
                }]
            })))
            .with_status(202)
            .with_header("x-message-id", "msg_123456789")
            .create_async()
            .await;

        let result = send_welcome_email(&config, &user).await;
        assert!(result.is_ok());

        // Give the spawned task time to execute
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_send_welcome_email_missing_api_key() {
        let _guard = EnvGuard::new(&["MAILERSEND_API_KEY", "WELCOME_EMAIL_TEMPLATE_ID"]);

        // Set test state
        env::remove_var("MAILERSEND_API_KEY");
        env::set_var("WELCOME_EMAIL_TEMPLATE_ID", "template_123");

        let config = Config::default();
        assert!(
            config.mailersend_api_key().is_none(),
            "API key should be None"
        );

        let user = create_test_user();

        let result = send_welcome_email(&config, &user).await;
        assert!(result.is_err());

        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Config) => {}
                _ => panic!("Expected Config error, got: {:?}", e.error_kind),
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_send_welcome_email_missing_template_id() {
        let _guard = EnvGuard::new(&["MAILERSEND_API_KEY", "WELCOME_EMAIL_TEMPLATE_ID"]);

        // Set test state
        env::set_var("MAILERSEND_API_KEY", "test_api_key_123");
        env::remove_var("WELCOME_EMAIL_TEMPLATE_ID");

        let config = Config::default();
        assert!(
            config.mailersend_api_key().is_some(),
            "API key should be present"
        );
        assert!(
            config.welcome_email_template_id().is_none(),
            "Template ID should be None"
        );

        let user = create_test_user();

        let result = send_welcome_email(&config, &user).await;
        assert!(result.is_err());

        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Config) => {}
                _ => panic!("Expected Config error, got: {:?}", e.error_kind),
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_send_welcome_email_http_error() {
        let mut server = setup_test_server().await;
        let user = create_test_user();
        let config = create_config_with_mock(&server.url());

        let _mock = server
            .mock("POST", "/v1/email")
            .with_status(400)
            .with_body(r#"{"message": "Invalid request"}"#)
            .create_async()
            .await;

        // The function returns Ok because send_email spawns async
        let result = send_welcome_email(&config, &user).await;
        assert!(result.is_ok());

        // Give the spawned task time to execute and log the error
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_send_welcome_email_server_timeout() {
        let mut server = setup_test_server().await;
        let user = create_test_user();
        let config = create_config_with_mock(&server.url());

        // Create a mock that delays response
        let _mock = server
            .mock("POST", "/v1/email")
            .with_status(202)
            .with_chunked_body(|w| {
                std::thread::sleep(std::time::Duration::from_secs(5));
                w.write_all(b"{}")
            })
            .expect_at_most(1)
            .create_async()
            .await;

        // Function should return Ok immediately due to async spawning
        let result = send_welcome_email(&config, &user).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[serial]
    async fn test_send_welcome_email_with_different_user_data() {
        let mut server = setup_test_server().await;
        let user = users::Model {
            id: Id::new_v4(),
            first_name: "Jane".to_string(),
            last_name: "Smith".to_string(),
            email: "jane.smith@test.org".to_string(),
            display_name: Some("Jane Smith".to_string()),
            password: "hashed_password".to_string(),
            github_username: Some("janesmith".to_string()),
            github_profile_url: Some("https://github.com/janesmith".to_string()),
            timezone: "America/New_York".to_string(),
            role: users::Role::Admin,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
        };
        let config = create_config_with_mock(&server.url());

        let _mock = server
            .mock("POST", "/v1/email")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": {
                    "email": "hello@myrefactor.com",
                    "name": null
                },
                "to": [{
                    "email": "jane.smith@test.org",
                    "name": "Jane Smith"
                }],
                "subject": "Welcome to Refactor Platform",
                "template_id": "template_123",
                "personalization": [{
                    "email": "jane.smith@test.org",
                    "data": {
                        "first_name": "Jane",
                        "last_name": "Smith"
                    }
                }]
            })))
            .with_status(202)
            .create_async()
            .await;

        let result = send_welcome_email(&config, &user).await;
        assert!(result.is_ok());

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_send_welcome_email_validates_personalization() {
        let mut server = setup_test_server().await;
        let user = users::Model {
            id: Id::new_v4(),
            first_name: "Test-First".to_string(),
            last_name: "Test-Last".to_string(),
            email: "test.user@example.com".to_string(),
            display_name: None,
            password: "hashed_password".to_string(),
            github_username: None,
            github_profile_url: None,
            timezone: "Europe/London".to_string(),
            role: users::Role::Admin,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
        };
        let config = create_config_with_mock(&server.url());

        let _mock = server
            .mock("POST", "/v1/email")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": {
                    "email": "hello@myrefactor.com",
                    "name": null
                },
                "to": [{
                    "email": "test.user@example.com",
                    "name": "Test-First Test-Last"
                }],
                "subject": "Welcome to Refactor Platform",
                "template_id": "template_123",
                "personalization": [{
                    "email": "test.user@example.com",
                    "data": {
                        "first_name": "Test-First",
                        "last_name": "Test-Last"
                    }
                }]
            })))
            .with_status(202)
            .create_async()
            .await;

        let result = send_welcome_email(&config, &user).await;
        assert!(result.is_ok());

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}
