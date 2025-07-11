use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use email_address::EmailAddress;
use log::*;
use serde::{Deserialize, Serialize};
use service::config::Config;

/// MailerSend API client for sending transactional emails
pub struct MailerSendClient {
    client: reqwest::Client,
    base_url: String,
}

/// Email recipient with name and email address
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailRecipient {
    pub email: String,
    pub name: Option<String>,
}

/// Email sender with name and email address
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailSender {
    pub email: String,
    pub name: Option<String>,
}

/// Request payload for sending an email via MailerSend
#[derive(Debug, Serialize)]
pub struct SendEmailRequest {
    pub from: EmailSender,
    pub to: Vec<EmailRecipient>,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<Vec<EmailRecipient>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc: Option<Vec<EmailRecipient>>,
}

/// Response from MailerSend API
#[derive(Debug, Deserialize)]
pub struct SendEmailResponse {
    pub message_id: Option<String>,
}

impl MailerSendClient {
    /// Create a new MailerSend client with authentication
    pub async fn new(config: &Config) -> Result<Self, Error> {
        let client = build_client(config).await?;
        let base_url = "https://api.mailersend.com/v1".to_string();

        Ok(Self { client, base_url })
    }

    /// Send an email using MailerSend API
    pub async fn send_email(&self, request: SendEmailRequest) -> Result<SendEmailResponse, Error> {
        // Validate email addresses before sending
        if !is_valid_email(&request.from.email) {
            warn!("Invalid sender email: {}", request.from.email);
            return Err(Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Invalid sender email address".to_string(),
                )),
            });
        }

        for recipient in &request.to {
            if !is_valid_email(&recipient.email) {
                warn!("Invalid recipient email: {}", recipient.email);
                return Err(Error {
                    source: None,
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(format!(
                        "Invalid recipient email address: {}",
                        recipient.email
                    ))),
                });
            }
        }

        let url = format!("{}/email", self.base_url);

        info!("Sending email to {} recipients", request.to.len());
        debug!("Email subject: {}", request.subject);

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
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        let status = response.status();
        if status.is_success() {
            let message_id = response
                .headers()
                .get("x-message-id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            info!("Email sent successfully, message_id: {:?}", message_id);

            Ok(SendEmailResponse { message_id })
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Failed to send email: {} - {}", status, error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            })
        }
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
    let auth_value = format!("Bearer {}", api_key);
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

/// Validate email address format using email_address crate
pub fn is_valid_email(email: &str) -> bool {
    EmailAddress::is_valid(email)
}

#[cfg(test)]
mod tests {
    use super::*;
    use service::config::Config;

    #[tokio::test]
    async fn test_mailersend_client_creation_fails_without_api_key() {
        let config = Config::default();
        let result = MailerSendClient::new(&config).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_send_email_request_serialization() {
        let request = SendEmailRequest {
            from: EmailSender {
                email: "test@example.com".to_string(),
                name: Some("Test Sender".to_string()),
            },
            to: vec![EmailRecipient {
                email: "recipient@example.com".to_string(),
                name: Some("Test Recipient".to_string()),
            }],
            subject: "Test Subject".to_string(),
            text: Some("Test email body".to_string()),
            html: None,
            cc: None,
            bcc: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("test@example.com"));
        assert!(json.contains("Test Subject"));
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
                !is_valid_email(email),
                "Email '{}' should be invalid",
                email
            );
        }

        // Test valid emails
        assert!(is_valid_email("test@example.com"));
        assert!(is_valid_email("user.name@domain.co.uk"));
    }
}
