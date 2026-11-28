//! Chooses the email provider: the one place that knows which provider sends outbox email.

use std::sync::Arc;

use log::warn;
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

/// The configured email provider, or `None` when none is configured or it failed to build,
/// which is logged.
pub async fn configured(config: &Config) -> Option<Arc<dyn Mailer>> {
    from_config(config)
        .await
        .inspect_err(|e| warn!("The email provider failed to build: {e:?}"))
        .ok()
        .flatten()
}
