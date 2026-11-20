//! Chooses the email provider: the one place that knows which provider sends outbox email.

use std::sync::Arc;

use service::config::Config;

use crate::email_outbox::Mailer;
use crate::error::Error;
use crate::gateway::resend;

/// The configured email provider, `None` when none is configured, or why it failed to build.
pub async fn from_config(config: &Config) -> Result<Option<Arc<dyn Mailer>>, Error> {
    match config.resend_api_key() {
        None => Ok(None),
        Some(_) => resend::Client::new(config)
            .await
            .map(|client| Some(Arc::new(client) as Arc<dyn Mailer>)),
    }
}
