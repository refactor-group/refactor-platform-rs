//! Zoom OAuth client.
//!
//! Provides a configured Zoom OAuth provider for domain controllers.

use meeting_auth::oauth::providers::zoom::Provider as ZoomProvider;
use secrecy::SecretString;

/// Create a new Zoom OAuth provider.
///
/// # Arguments
///
/// * `client_id` - Zoom OAuth client ID from config
/// * `client_secret` - Zoom OAuth client secret from config
/// * `redirect_uri` - OAuth redirect URI from config
///
/// # Returns
///
/// `Ok(ZoomProvider)` on success, or `Err(meeting_auth::error::Error)` if the
/// underlying rustls HTTP client cannot be constructed.
///
/// # Example
///
/// ```rust,ignore
/// use domain::gateway::oauth::zoom;
///
/// let provider = zoom::new_provider(
///     config.zoom_client_id().unwrap(),
///     SecretString::from(config.zoom_client_secret().unwrap()),
///     config.zoom_redirect_uri().unwrap(),
/// )
/// .expect("failed to build Zoom OAuth provider");
/// ```
pub fn new_provider(
    client_id: String,
    client_secret: SecretString,
    redirect_uri: String,
) -> Result<ZoomProvider, meeting_auth::error::Error> {
    ZoomProvider::new(client_id, client_secret, redirect_uri)
        .map_err(meeting_auth::error::Error::from)
}
