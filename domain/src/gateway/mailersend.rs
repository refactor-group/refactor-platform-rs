use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use email_address::EmailAddress;
use log::*;
use serde::{Deserialize, Serialize};
use service::config::Config;
use std::collections::HashMap;
use std::fmt::Display;

/// Template ID with validation
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TemplateId(String);

impl TemplateId {
    pub fn new(id: impl Into<String>) -> Result<Self, Error> {
        let id = id.into();
        if id.is_empty() {
            return Err(Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
            });
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<TemplateId> for String {
    fn from(template_id: TemplateId) -> String {
        template_id.0
    }
}

impl Serialize for TemplateId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

/// MailerSend API client for sending transactional emails
pub struct MailerSendClient {
    client: reqwest::Client,
    base_url: String,
}

/// Email recipient with name and email address
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct EmailRecipient {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct Personalization {
    pub email: String,
    pub data: HashMap<String, String>,
}

/// Request payload for sending an email via MailerSend
#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct SendEmailRequest {
    pub from: EmailRecipient,
    pub to: Vec<EmailRecipient>,
    pub subject: String,
    pub template_id: Option<TemplateId>,
    pub personalization: Vec<Personalization>,
}

/// Builder for constructing SendEmailRequest with fluent API
///
/// This builder provides multiple methods for constructing email requests.
/// All methods are part of the public API and may be used by consumers of this library.
pub struct SendEmailRequestBuilder {
    from: Option<EmailRecipient>,
    to: Vec<EmailRecipient>,
    subject: Option<String>,
    template_id: Option<String>,
    personalization: HashMap<String, String>,
}

/// Response from MailerSend API
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SendEmailResponse {
    pub message_id: Option<String>,
}

impl SendEmailRequestBuilder {
    /// Create a new builder with the required template ID
    pub fn new() -> Self {
        SendEmailRequestBuilder {
            from: None,
            to: Vec::new(),
            subject: None,
            template_id: None,
            personalization: HashMap::new(),
        }
    }

    /// Set the sender email address
    pub fn from(mut self, email: impl Into<String>) -> Self {
        self.from = Some(EmailRecipient {
            email: email.into(),
            name: None,
        });
        self
    }

    /// Add a recipient with name  
    #[allow(clippy::wrong_self_convention)] // Builder pattern convention
    pub fn to_with_name(mut self, email: impl Into<String>, name: impl Into<String>) -> Self {
        self.to.push(EmailRecipient {
            email: email.into(),
            name: Some(name.into()),
        });
        self
    }

    /// Set the email subject
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Set the template ID
    pub fn template_id(mut self, template_id: String) -> Self {
        self.template_id = Some(template_id);
        self
    }

    /// Add a personalization variable
    pub fn add_personalization(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.personalization.insert(key.into(), value.into());
        self
    }

    /// Build the SendEmailRequest with validation
    pub async fn build(self) -> Result<SendEmailRequest, Error> {
        // Validate from field
        let from = self.from.ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Sender email is required".to_string(),
            )),
        })?;
        SendEmailRequest::validate_email(&from.email)?;

        // Validate recipients
        if self.to.is_empty() {
            return Err(Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "At least one recipient is required".to_string(),
                )),
            });
        }

        for recipient in &self.to {
            SendEmailRequest::validate_email(&recipient.email)?;
        }

        // Validate subject
        let subject = self.subject.ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Subject is required".to_string(),
            )),
        })?;

        // Template ID is already validated when set
        let template_id = if let Some(id) = self.template_id {
            Some(TemplateId::new(id)?)
        } else {
            None
        };

        // Create personalization for each recipient if data exists
        let personalization = if !self.personalization.is_empty() {
            self.to
                .iter()
                .map(|recipient| Personalization {
                    email: recipient.email.clone(),
                    data: self.personalization.clone(),
                })
                .collect()
        } else {
            vec![]
        };

        Ok(SendEmailRequest {
            from,
            to: self.to,
            subject,
            template_id,
            personalization,
        })
    }
}

impl SendEmailRequest {
    /// Validate email address and return error if invalid
    fn validate_email<E>(email: E) -> Result<(), Error>
    where
        E: AsRef<str> + Display,
    {
        if !EmailAddress::is_valid(email.as_ref()) {
            warn!("Invalid email: {email}");
            return Err(Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(format!(
                    "Invalid email address: {email}"
                ))),
            });
        }
        Ok(())
    }
}

impl MailerSendClient {
    /// Create a new MailerSend client with authentication
    pub async fn new(config: &Config) -> Result<Self, Error> {
        let client = build_client(config).await?;
        let base_url = "https://api.mailersend.com/v1".to_string();

        Ok(Self { client, base_url })
    }

    /// Send an email using MailerSend API asynchronously
    ///
    /// This method spawns the email sending operation in a background task,
    /// allowing the caller to continue without waiting for the email to be sent.
    /// The result of the email sending is logged but not returned to the caller.
    pub async fn send_email(&self, request: SendEmailRequest) {
        // Clone values needed for the async task
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let to_count = request.to.len();
        let to_emails: Vec<String> = request.to.iter().map(|r| r.email.clone()).collect();

        info!("Queuing email for {to_count} recipients");

        // Spawn the email sending as a background task
        tokio::spawn(async move {
            let url = format!("{base_url}/email");

            info!("Sending email to {to_count} recipients: {to_emails:?}");

            let response = match client.post(&url).json(&request).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    warn!("Failed to send email request: {e:?}");
                    return Err(Error {
                        source: Some(Box::new(e)),
                        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                            "Failed to send email request".to_string(),
                        )),
                    });
                }
            };

            let status = response.status();
            if status.is_success() {
                let message_id = response
                    .headers()
                    .get("x-message-id")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());

                info!("Email sent successfully to {to_emails:?}, message_id: {message_id:?}");
                Ok(())
            } else {
                let error_text = response.text().await.unwrap_or_default();
                warn!("Failed to send email to {to_emails:?}: {status} - {error_text}");
                Err(Error {
                    source: None,
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(error_text)),
                })
            }
        });
    }
}

/// Build HTTP client with MailerSend authentication
async fn build_client(config: &Config) -> Result<reqwest::Client, Error> {
    let headers = build_auth_headers(config).await?;

    Ok(reqwest::Client::builder()
        .use_rustls_tls()
        .default_headers(headers)
        .build()?)
}

/// Build authentication headers for MailerSend API
async fn build_auth_headers(config: &Config) -> Result<reqwest::header::HeaderMap, Error> {
    let api_key = config.mailersend_api_key().ok_or_else(|| {
        warn!("Failed to get MailerSend API key from config");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    let mut headers = reqwest::header::HeaderMap::new();
    let auth_value = format!("Bearer {api_key}");
    let mut auth_header = reqwest::header::HeaderValue::from_str(&auth_value).map_err(|err| {
        warn!("Failed to create authorization header value: {err:?}");
        Error {
            source: Some(Box::new(err)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Failed to create authorization header value".to_string(),
            )),
        }
    })?;
    auth_header.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, auth_header);

    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use service::config::Config;

    #[tokio::test]
    #[serial]
    async fn test_mailersend_client_creation_fails_without_api_key() {
        // Save current env state
        let saved_api_key = std::env::var("MAILERSEND_API_KEY").ok();

        // Ensure no API key is set
        std::env::remove_var("MAILERSEND_API_KEY");

        let config = Config::default();
        assert!(
            config.mailersend_api_key().is_none(),
            "API key should be None"
        );

        let result = MailerSendClient::new(&config).await;
        assert!(result.is_err());

        // Restore env state
        if let Some(key) = saved_api_key {
            std::env::set_var("MAILERSEND_API_KEY", key);
        }
    }

    #[tokio::test]
    async fn test_send_email_request_serialization() {
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("recipient@example.com", "Test Recipient")
            .subject("Test Subject")
            .add_personalization("name", "Test User")
            .build()
            .await
            .unwrap();

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("recipient@example.com"));
    }

    #[tokio::test]
    async fn test_send_email_request_with_personalization() {
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("john.doe@example.com", "John Doe")
            .to_with_name("jane.smith@example.com", "Jane Smith")
            .subject("Test Subject")
            .add_personalization("first_name", "John")
            .add_personalization("last_name", "Doe")
            .add_personalization("company", "Acme Corp")
            .build()
            .await
            .unwrap();

        // Verify personalization for both recipients
        assert_eq!(request.personalization.len(), 2);
        assert_eq!(request.personalization[0].email, "john.doe@example.com");
        assert_eq!(request.personalization[1].email, "jane.smith@example.com");
        assert_eq!(
            request.personalization[0].data.get("first_name"),
            Some(&"John".to_string())
        );
        assert_eq!(
            request.personalization[0].data.get("last_name"),
            Some(&"Doe".to_string())
        );
        assert_eq!(
            request.personalization[0].data.get("company"),
            Some(&"Acme Corp".to_string())
        );
        assert_eq!(request.to.len(), 2);
    }

    #[tokio::test]
    async fn test_send_email_request_empty_recipients_fails() {
        let result = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .subject("Test Subject")
            .build()
            .await;

        assert!(result.is_err());
    }

    #[test]
    fn test_email_validation() {
        // Test invalid email formats
        let invalid_emails = vec![
            "",
            "invalid-email",
            "@example.com",
            "test@",
            "test..test@example.com",
        ];

        for email in invalid_emails {
            assert!(
                SendEmailRequest::validate_email(email).is_err(),
                "Email '{}' should be invalid",
                email
            );
        }

        // Test valid emails
        assert!(SendEmailRequest::validate_email("test@example.com").is_ok());
        assert!(SendEmailRequest::validate_email("user.name@domain.co.uk").is_ok());
    }

    #[tokio::test]
    async fn test_builder_multiple_recipients() {
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("first@example.com", "First User")
            .to_with_name("second@example.com", "Second User")
            .subject("Multi Recipient")
            .build()
            .await
            .unwrap();

        assert_eq!(request.to.len(), 2);
        assert_eq!(request.to[0].email, "first@example.com");
        assert_eq!(request.to[1].email, "second@example.com");
    }

    #[tokio::test]
    async fn test_builder_missing_from_fails() {
        let result = SendEmailRequestBuilder::new()
            .to_with_name("recipient@example.com", "Test User")
            .subject("Test")
            .build()
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{:?}", err).contains("Sender email is required"));
    }

    #[tokio::test]
    async fn test_builder_missing_recipients_fails() {
        let result = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .subject("Test")
            .build()
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{:?}", err).contains("At least one recipient is required"));
    }

    #[tokio::test]
    async fn test_builder_missing_subject_fails() {
        let result = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("recipient@example.com", "Test User")
            .build()
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{:?}", err).contains("Subject is required"));
    }

    #[tokio::test]
    async fn test_builder_invalid_from_email_fails() {
        let result = SendEmailRequestBuilder::new()
            .from("invalid-email")
            .to_with_name("recipient@example.com", "Test User")
            .subject("Test")
            .build()
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_builder_invalid_recipient_email_fails() {
        let result = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("", "Test User")
            .subject("Test")
            .build()
            .await;

        assert!(result.is_err());
    }
}
