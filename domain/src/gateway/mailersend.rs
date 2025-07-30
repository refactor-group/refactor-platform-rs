use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use email_address::EmailAddress;
use log::*;
use serde::{Deserialize, Serialize};
use service::config::Config;
use std::collections::HashMap;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Personalization {
    pub email: String,
    pub data: HashMap<String, String>,
}

/// Request payload for sending an email via MailerSend
#[derive(Debug, Serialize)]
pub struct SendEmailRequest {
    pub from: EmailRecipient,
    pub to: Vec<EmailRecipient>,
    pub subject: String,
    pub template_id: String,
    pub personalization: Vec<Personalization>,
}

/// Response from MailerSend API
#[derive(Debug, Deserialize)]
pub struct SendEmailResponse {
    pub message_id: Option<String>,
}

impl SendEmailRequest {
    pub async fn new(
        from: EmailRecipient,
        to: Vec<EmailRecipient>,
        subject: String,
        template_id: String,
        personalization_data: HashMap<String, String>,
    ) -> Result<Self, Error> {
        // Validate from email
        Self::validate_email(&from.email)?;

        // Validate recipient emails
        for recipient in &to {
            Self::validate_email(&recipient.email)?;
        }

        // Get the first recipient's email for personalization
        let email = to
            .first()
            .ok_or_else(|| Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "At least one recipient is required".to_string(),
                )),
            })?
            .email
            .clone();

        Ok(SendEmailRequest {
            from,
            to: to.clone(),
            subject,
            template_id,
            personalization: vec![Personalization {
                email,
                data: personalization_data,
            }],
        })
    }

    /// Validate email address and return error if invalid
    fn validate_email(email: &str) -> Result<(), Error> {
        if !EmailAddress::is_valid(email) {
            warn!("Invalid email: {}", email);
            return Err(Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(format!(
                    "Invalid email address: {}",
                    email
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

    /// Send an email using MailerSend API
    pub async fn send_email(&self, request: SendEmailRequest) -> Result<SendEmailResponse, Error> {
        let url = format!("{}/email", self.base_url);

        info!("Sending email to {} recipients", request.to.len());

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

    #[tokio::test]
    async fn test_send_email_request_serialization() {
        let mut personalization_data = HashMap::new();
        personalization_data.insert("name".to_string(), "Test User".to_string());

        let request = SendEmailRequest::new(
            EmailRecipient {
                email: "sender@example.com".to_string(),
                name: Some("Test Sender".to_string()),
            },
            vec![EmailRecipient {
                email: "recipient@example.com".to_string(),
                name: Some("Test Recipient".to_string()),
            }],
            "Test Subject".to_string(),
            "x8emy5o5world01w".to_string(),
            personalization_data,
        )
        .await
        .unwrap();

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("recipient@example.com"));
        assert!(json.contains("x8emy5o5world01w"));
    }

    #[tokio::test]
    async fn test_send_email_request_with_personalization() {
        let mut personalization_data = HashMap::new();
        personalization_data.insert("first_name".to_string(), "John".to_string());
        personalization_data.insert("last_name".to_string(), "Doe".to_string());
        personalization_data.insert("company".to_string(), "Acme Corp".to_string());

        let recipients = vec![
            EmailRecipient {
                email: "john.doe@example.com".to_string(),
                name: Some("John Doe".to_string()),
            },
            EmailRecipient {
                email: "jane.smith@example.com".to_string(),
                name: Some("Jane Smith".to_string()),
            },
        ];

        let request = SendEmailRequest::new(
            EmailRecipient {
                email: "sender@example.com".to_string(),
                name: Some("Test Sender".to_string()),
            },
            recipients,
            "Test Subject".to_string(),
            "template123".to_string(),
            personalization_data.clone(),
        )
        .await
        .unwrap();

        // Verify personalization uses the first recipient's email
        assert_eq!(request.personalization.len(), 1);
        assert_eq!(request.personalization[0].email, "john.doe@example.com");
        assert_eq!(request.personalization[0].data, personalization_data);
        assert_eq!(request.to.len(), 2);
    }

    #[tokio::test]
    async fn test_send_email_request_empty_recipients_fails() {
        let personalization_data = HashMap::new();

        let result = SendEmailRequest::new(
            EmailRecipient {
                email: "sender@example.com".to_string(),
                name: Some("Test Sender".to_string()),
            },
            vec![],
            "Test Subject".to_string(),
            "template123".to_string(),
            personalization_data,
        )
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
}
