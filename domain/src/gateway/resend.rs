use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use email_address::EmailAddress;
use log::*;
use serde::{Deserialize, Serialize, Serializer};
use service::config::Config;
use std::collections::HashMap;
use std::fmt::Display;

/// Path appended to the configured base URL when sending emails.
const SEND_EMAIL_PATH: &str = "/emails";

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
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

/// Resend API client for sending transactional emails
pub struct Client {
    client: reqwest::Client,
    base_url: String,
}

/// Email recipient with optional display name.
///
/// Resend accepts recipients as RFC 5322 mailbox strings (`Name <email>` or just `email`),
/// so this serializes to a single string regardless of whether a name is set.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EmailRecipient {
    pub email: String,
    pub name: Option<String>,
}

impl Serialize for EmailRecipient {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.name {
            Some(name) if !name.is_empty() => {
                serializer.serialize_str(&format!("{name} <{}>", self.email))
            }
            _ => serializer.serialize_str(&self.email),
        }
    }
}

/// Reference to a published Resend template plus the variables to interpolate.
#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct TemplateRef {
    pub id: TemplateId,
    pub variables: HashMap<String, String>,
}

/// Request payload for sending an email via Resend.
#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct SendEmailRequest {
    pub from: EmailRecipient,
    pub to: Vec<EmailRecipient>,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateRef>,
}

/// Builder for constructing SendEmailRequest with fluent API
pub struct SendEmailRequestBuilder {
    from: Option<EmailRecipient>,
    to: Vec<EmailRecipient>,
    subject: Option<String>,
    template_id: Option<String>,
    variables: HashMap<String, String>,
}

/// Response from Resend's `POST /emails` endpoint.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SendEmailResponse {
    pub id: Option<String>,
}

impl Default for SendEmailRequestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SendEmailRequestBuilder {
    pub fn new() -> Self {
        SendEmailRequestBuilder {
            from: None,
            to: Vec::new(),
            subject: None,
            template_id: None,
            variables: HashMap::new(),
        }
    }

    /// Set the sender email address.
    pub fn from(mut self, email: impl Into<String>) -> Self {
        self.from = Some(EmailRecipient {
            email: email.into(),
            name: None,
        });
        self
    }

    /// Add a recipient with display name.
    #[allow(clippy::wrong_self_convention)] // Builder pattern convention
    pub fn to_with_name(mut self, email: impl Into<String>, name: impl Into<String>) -> Self {
        self.to.push(EmailRecipient {
            email: email.into(),
            name: Some(name.into()),
        });
        self
    }

    /// Set the email subject.
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Set the Resend template ID (or published alias).
    pub fn template_id(mut self, template_id: impl Into<String>) -> Self {
        self.template_id = Some(template_id.into());
        self
    }

    /// Add a template variable for interpolation by Resend.
    pub fn add_variable(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.variables.insert(key.into(), value.into());
        self
    }

    /// Build the SendEmailRequest with validation.
    pub async fn build(self) -> Result<SendEmailRequest, Error> {
        let from = self.from.ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Sender email is required".to_string(),
            )),
        })?;
        SendEmailRequest::validate_email(&from.email)?;

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

        let subject = self.subject.ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Subject is required".to_string(),
            )),
        })?;

        let template = match self.template_id {
            Some(id) => Some(TemplateRef {
                id: TemplateId::new(id)?,
                variables: self.variables,
            }),
            None => None,
        };

        Ok(SendEmailRequest {
            from,
            to: self.to,
            subject,
            template,
        })
    }
}

impl SendEmailRequest {
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

impl Client {
    /// Create a new Resend client with authentication headers preconfigured.
    pub async fn new(config: &Config) -> Result<Self, Error> {
        let client = build_client(config).await?;
        let base_url = config.resend_base_url().to_string();
        Ok(Self { client, base_url })
    }

    /// Send an email via Resend's `POST /emails` endpoint.
    pub async fn send_email(&self, request: SendEmailRequest) -> Result<(), Error> {
        let url = format!("{}{SEND_EMAIL_PATH}", self.base_url);
        let to_emails: Vec<String> = request.to.iter().map(|r| r.email.clone()).collect();

        info!(
            "Sending email to {} recipients: {to_emails:?}",
            request.to.len()
        );

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to send email request: {e:?}");
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                        "Failed to send email request".to_string(),
                    )),
                }
            })?;

        let status = response.status();
        if status.is_success() {
            let parsed: SendEmailResponse = response
                .json()
                .await
                .unwrap_or(SendEmailResponse { id: None });
            info!(
                "Email sent successfully to {to_emails:?}, id: {:?}",
                parsed.id
            );
            Ok(())
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Failed to send email to {to_emails:?}: {status} - {error_text}");
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(error_text)),
            })
        }
    }
}

/// Build HTTP client with Resend authentication headers preconfigured.
async fn build_client(config: &Config) -> Result<reqwest::Client, Error> {
    let headers = build_auth_headers(config).await?;

    Ok(reqwest::Client::builder()
        .use_rustls_tls()
        .default_headers(headers)
        .build()?)
}

/// Build authentication headers for the Resend API.
async fn build_auth_headers(config: &Config) -> Result<reqwest::header::HeaderMap, Error> {
    let api_key = config.resend_api_key().ok_or_else(|| {
        warn!("Failed to get Resend API key from config");
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
    use service::config::Config;

    #[tokio::test]
    async fn test_client_creation_fails_without_api_key() {
        let config = Config::default();
        assert!(config.resend_api_key().is_none(), "API key should be None");

        let result = Client::new(&config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_email_request_serialization() {
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("recipient@example.com", "Test Recipient")
            .subject("Test Subject")
            .template_id("welcome-email")
            .add_variable("name", "Test User")
            .build()
            .await
            .unwrap();

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("Test Recipient <recipient@example.com>"));
        assert!(json.contains("\"template\""));
        assert!(json.contains("\"welcome-email\""));
    }

    #[tokio::test]
    async fn test_send_email_request_with_variables() {
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("john.doe@example.com", "John Doe")
            .to_with_name("jane.smith@example.com", "Jane Smith")
            .subject("Test Subject")
            .template_id("multi-recipient-template")
            .add_variable("first_name", "John")
            .add_variable("last_name", "Doe")
            .add_variable("company", "Acme Corp")
            .build()
            .await
            .unwrap();

        assert_eq!(request.to.len(), 2);
        let template = request.template.as_ref().expect("template should be set");
        assert_eq!(template.id.as_str(), "multi-recipient-template");
        assert_eq!(
            template.variables.get("first_name"),
            Some(&"John".to_string())
        );
        assert_eq!(
            template.variables.get("last_name"),
            Some(&"Doe".to_string())
        );
        assert_eq!(
            template.variables.get("company"),
            Some(&"Acme Corp".to_string())
        );
    }

    #[tokio::test]
    async fn test_send_email_recipient_without_name_serializes_as_bare_email() {
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("recipient@example.com", "")
            .subject("Test Subject")
            .template_id("t")
            .build()
            .await
            .unwrap();

        let json = serde_json::to_string(&request).unwrap();
        // Empty name → bare email, not `" <recipient@example.com>"`
        assert!(json.contains("\"to\":[\"recipient@example.com\"]"));
        assert!(json.contains("\"from\":\"sender@example.com\""));
    }

    #[tokio::test]
    async fn test_send_email_request_empty_recipients_fails() {
        let result = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .subject("Test Subject")
            .template_id("t")
            .build()
            .await;

        assert!(result.is_err());
    }

    #[test]
    fn test_email_validation() {
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
                "Email '{email}' should be invalid"
            );
        }

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
            .template_id("t")
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
        assert!(format!("{err:?}").contains("Sender email is required"));
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
        assert!(format!("{err:?}").contains("At least one recipient is required"));
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
        assert!(format!("{err:?}").contains("Subject is required"));
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

    #[test]
    fn test_send_email_path_constant() {
        assert_eq!(SEND_EMAIL_PATH, "/emails");
    }
}
