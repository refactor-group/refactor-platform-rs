use std::collections::HashMap;
use std::fmt::Display;

use email_address::EmailAddress;
use log::*;
use serde::{Deserialize, Serialize, Serializer};
use service::config::Config;

use crate::error::{DomainErrorKind, Error, InternalErrorKind};

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

/// Format a recipient as an RFC 5322 mailbox string.
///
/// The display name is wrapped in a quoted-string and its `\` and `"` are
/// backslash-escaped. Names come from user-supplied first/last name fields and
/// may contain "specials" (comma, quote, angle brackets, etc.); quoting
/// unconditionally is always valid and avoids brittle specials-detection.
fn format_mailbox(name: &str, email: &str) -> String {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\" <{email}>")
}

impl Serialize for EmailRecipient {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.name {
            Some(name) if !name.is_empty() => {
                serializer.serialize_str(&format_mailbox(name, &self.email))
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
///
/// `subject` is optional because Resend allows the subject to be authored on
/// the template itself. When omitted from the payload, Resend uses the
/// template's default subject. When provided, the payload value overrides the
/// template default (per Resend's "payload wins over template defaults" rule).
#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct SendEmailRequest {
    pub from: EmailRecipient,
    pub to: Vec<EmailRecipient>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateRef>,
}

/// Builder for constructing SendEmailRequest with fluent API
pub struct SendEmailRequestBuilder {
    from: Option<EmailRecipient>,
    to: Vec<EmailRecipient>,
    template_id: Option<String>,
    variables: HashMap<String, String>,
}

/// Response from Resend's `POST /emails` endpoint.
#[derive(Debug, Deserialize)]
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

    /// Add a template variable only when `value` is `Some`.
    ///
    /// `None` omits the key from the payload so Resend's template
    /// `fallback_value` can fire — sending an empty string would defeat it.
    pub fn add_optional_variable(mut self, key: impl Into<String>, value: Option<&str>) -> Self {
        if let Some(v) = value {
            self.variables.insert(key.into(), v.to_string());
        }
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
            subject: None,
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
            .template_id("welcome-email")
            .add_variable("name", "Test User")
            .build()
            .await
            .unwrap();

        // Structural JSON equality — a substring check can't catch a misplaced
        // or misnamed field the way full-shape comparison does.
        let actual: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        let expected = serde_json::json!({
            "from": "sender@example.com",
            "to": ["\"Test Recipient\" <recipient@example.com>"],
            "template": {
                "id": "welcome-email",
                "variables": { "name": "Test User" }
            }
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_mailbox_quotes_and_escapes_specials() {
        // A comma would split into two malformed mailboxes if left unquoted.
        assert_eq!(
            format_mailbox("Smith, Jr.", "jr@example.com"),
            r#""Smith, Jr." <jr@example.com>"#
        );
        // Literal backslash and quote must be backslash-escaped inside the
        // quoted-string. Input name is: He said "hi"\
        assert_eq!(
            format_mailbox(r#"He said "hi"\"#, "x@example.com"),
            r#""He said \"hi\"\\" <x@example.com>"#
        );
    }

    #[tokio::test]
    async fn test_send_email_request_with_variables() {
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("john.doe@example.com", "John Doe")
            .to_with_name("jane.smith@example.com", "Jane Smith")
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
            .template_id("t")
            .build()
            .await
            .unwrap();

        let json = serde_json::to_string(&request).unwrap();
        // Empty name → bare email, not `" <recipient@example.com>"`
        assert!(json.contains("\"to\":[\"recipient@example.com\"]"));
        assert!(json.contains("\"from\":\"sender@example.com\""));
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
            .build()
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{err:?}").contains("At least one recipient is required"));
    }

    #[tokio::test]
    async fn test_builder_omits_subject_when_not_set() {
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("recipient@example.com", "Test User")
            .template_id("t")
            .build()
            .await
            .unwrap();

        assert_eq!(request.subject, None);

        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        assert!(
            json.get("subject").is_none(),
            "subject must be omitted from JSON when not set, got: {json}"
        );
    }

    #[tokio::test]
    async fn test_builder_invalid_from_email_fails() {
        let result = SendEmailRequestBuilder::new()
            .from("invalid-email")
            .to_with_name("recipient@example.com", "Test User")
            .build()
            .await;

        let err = result.unwrap_err();
        assert!(
            format!("{err:?}").contains("Invalid email address"),
            "expected an invalid-email error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_builder_invalid_recipient_email_fails() {
        let result = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("", "Test User")
            .build()
            .await;

        let err = result.unwrap_err();
        assert!(
            format!("{err:?}").contains("Invalid email address"),
            "expected an invalid-email error, got: {err:?}"
        );
    }

    // ── Client::send_email HTTP behavior ───────────────────────────────

    /// Build a `Client` pointed at a mockito server, plus a minimal valid request.
    async fn client_and_request(server_url: &str) -> (Client, SendEmailRequest) {
        let config = Config::from_args([
            "test",
            "--resend-api-key=test_key",
            &format!("--resend-base-url={server_url}"),
        ]);
        let client = Client::new(&config).await.unwrap();
        let request = SendEmailRequestBuilder::new()
            .from("sender@example.com")
            .to_with_name("recipient@example.com", "Recipient")
            .template_id("t")
            .build()
            .await
            .unwrap();
        (client, request)
    }

    #[tokio::test]
    async fn test_client_send_email_posts_to_emails_path_on_success() {
        let mut server = mockito::Server::new_async().await;
        // Mock is registered for `/emails`; if `send_email` composed the wrong
        // URL, no mock would match and `assert_async` would fail.
        let mock = server
            .mock("POST", "/emails")
            .match_header("authorization", "Bearer test_key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"email_abc"}"#)
            .expect(1)
            .create_async()
            .await;

        let (client, request) = client_and_request(&server.url()).await;
        let result = client.send_email(request).await;

        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_client_send_email_non_2xx_is_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/emails")
            .with_status(422)
            .with_body(r#"{"message":"validation failed"}"#)
            .expect(1)
            .create_async()
            .await;

        let (client, request) = client_and_request(&server.url()).await;
        let result = client.send_email(request).await;

        // The response body must be folded into the error, not discarded —
        // it's the only diagnostic the caller gets for a rejected send.
        let err = result.unwrap_err();
        match err.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Other(text)) => assert!(
                text.contains("validation failed"),
                "response body not propagated into error, got: {text}"
            ),
            other => panic!("expected Internal(Other), got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_client_send_email_tolerates_non_json_success_body() {
        // A 2xx with a body that is not valid JSON must still succeed —
        // exercises the `unwrap_or(SendEmailResponse { id: None })` fallback.
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/emails")
            .with_status(200)
            .with_body("")
            .expect(1)
            .create_async()
            .await;

        let (client, request) = client_and_request(&server.url()).await;
        let result = client.send_email(request).await;

        assert!(result.is_ok());
    }
}
