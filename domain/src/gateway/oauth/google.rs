//! Google OAuth client.
//!
//! Provides a configured Google OAuth provider for domain controllers.

use meeting_auth::oauth::providers::google::Provider as GoogleProvider;

/// Create a new Google OAuth provider.
///
/// # Arguments
///
/// * `client_id` - Google OAuth client ID from config
/// * `client_secret` - Google OAuth client secret from config
/// * `redirect_uri` - OAuth redirect URI from config
///
/// # Returns
///
/// A configured Google OAuth provider ready for use.
///
/// # Example
///
/// ```rust,ignore
/// use domain::gateway::oauth::google;
///
/// let provider = google::new_provider(
///     config.google_client_id().unwrap(),
///     config.google_client_secret().unwrap(),
///     config.google_redirect_uri().unwrap(),
/// );
/// ```
pub fn new_provider(
    client_id: String,
    client_secret: String,
    redirect_uri: String,
) -> GoogleProvider {
    GoogleProvider::new(client_id, client_secret, redirect_uri)
}
