//! Error types for meeting AI operations.

use std::fmt;

/// Universal error type that abstracts provider-specific errors into common variants.
///
/// This unified error type eliminates the need for controller-level error mapping
/// and provides consistent error handling across all meeting AI providers.
/// All provider implementations should map their native errors to these variants,
/// preserving context while maintaining a provider-agnostic interface.
#[derive(Debug)]
pub enum Error {
    /// OAuth or API key authentication failures. Indicates credentials are invalid,
    /// expired, or lack necessary permissions. Clients should prompt for re-authentication.
    Authentication(String),

    /// Network connectivity issues, DNS failures, or connection timeouts.
    /// These errors are typically transient and may benefit from retry logic.
    Network(String),

    /// Invalid parameters, missing required fields, or malformed configuration.
    /// These errors indicate a programming error and should be fixed at development time.
    Configuration(String),

    /// Provider-specific business logic errors (e.g., meeting not found, bot rejected).
    /// These are provider-level failures that may require user intervention or workflow changes.
    Provider(String),

    /// Operation exceeded the configured or provider-enforced timeout period.
    /// Consider increasing timeout limits or breaking operations into smaller chunks.
    Timeout(String),

    /// Requested resource (bot, transcription, meeting) does not exist.
    /// Verify IDs are correct and the resource hasn't been deleted.
    NotFound(String),

    /// Provider rate limit exceeded. Clients must wait before retrying.
    /// Respect the retry_after_seconds to avoid further rate limiting or API suspension.
    RateLimited { retry_after_seconds: u64 },

    /// Failed to serialize data to JSON. Indicates type incompatibility or invalid data.
    /// Usually occurs when adding custom resources to AnalysisResult.
    Serialization(String),

    /// Failed to deserialize JSON data to expected type. Indicates type mismatch.
    /// Usually occurs when extracting resources with get_resources::<T>().
    Deserialization(String),

    /// Catch-all for errors that don't fit other categories.
    /// Used for unexpected errors or provider-specific edge cases.
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Authentication(msg) => write!(f, "Authentication failed: {}", msg),
            Error::Network(msg) => write!(f, "Network error: {}", msg),
            Error::Configuration(msg) => write!(f, "Invalid configuration: {}", msg),
            Error::Provider(msg) => write!(f, "Provider error: {}", msg),
            Error::Timeout(msg) => write!(f, "Timeout: {}", msg),
            Error::NotFound(msg) => write!(f, "Not found: {}", msg),
            Error::RateLimited {
                retry_after_seconds,
            } => {
                write!(f, "Rate limited: retry after {}s", retry_after_seconds)
            }
            Error::Serialization(msg) => write!(f, "Serialization error: {}", msg),
            Error::Deserialization(msg) => write!(f, "Deserialization error: {}", msg),
            Error::Other(err) => write!(f, "Other error: {}", err),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Other(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}
