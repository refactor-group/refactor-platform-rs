//! Error types for the `domain` layer.
use entity_api::error::{EntityApiErrorKind, Error as EntityApiError};
use meeting_auth::error::{
    Error as MeetingAuthError, ErrorKind as MeetingAuthErrorKind, OAuthErrorKind,
};
use std::error::Error as StdError;
use std::fmt;

/// Top-level domain error type.
/// Errors in the Domain layer are modeled as a tree structure
/// with `domain::error::Error` as the root type holding a tree of `error_kind`
/// enums that represent the kinds of errors that can occur in the domain layer or
/// in lower layers. The `source` field is used to hold the original error that caused
/// the domain error. The intent is to translate errors between layers while maintaining
/// layer boundaries. Ex. `domain` is dependent on `entity_api`, and `web` is dependent on `domain`.
/// but `web` should not be dependent, directly, on `entity_api`. Each layer is free to define its own
/// error kinds to whatever richeness needed at that layer. Ultimately the various `error_kind`s are used
/// by `web` to return appropriate HTTP status codes and messages to the client.
#[derive(Debug)]
pub struct Error {
    pub source: Option<Box<dyn StdError + Send + Sync>>,
    pub error_kind: DomainErrorKind,
}

/// Enum representing the major categories of errors that can occur in the `domain` layer.
#[derive(Debug, PartialEq)]
pub enum DomainErrorKind {
    Internal(InternalErrorKind),
    External(ExternalErrorKind),
    Validation(String),
}
/// Enum representing the various kinds of internal errors that can occur in the `domain` layer.
#[derive(Debug, PartialEq)]
pub enum InternalErrorKind {
    Entity(EntityErrorKind),
    Config,
    Other(String),
}

/// Enum representing the various kinds of entity errors that can bubble up from the "Entity" layer (`entity_api` and `entity`).
/// These errors are translated from the `entity_api` layer to the `domain` layer and reduced to a subset of error kinds
/// that are relevant to the `domain` layer.
#[derive(Debug, PartialEq)]
pub enum EntityErrorKind {
    NotFound,
    Invalid,
    Unauthenticated,
    Conflict {
        message: String,
        details: Option<serde_json::Value>,
    },
    CannotLinkCompletedGoal,
    GoalAlreadyLinkedToSession,
    /// Token missing, expired, or has wrong purpose. Collapsed deliberately
    /// for password-reset endpoints so attackers can't distinguish these
    /// three cases via the response.
    InvalidOrExpiredToken,
    /// User has exceeded the per-email password-reset request rate limit.
    PasswordResetRateLimited,
    DbTransaction,
    ServiceUnavailable,
    Other(String),
}

/// Enum representing the various kinds of external errors that can occur in the `domain`` layer.
#[derive(Debug, PartialEq)]
pub enum ExternalErrorKind {
    Network,
    OauthTokenRevoked(String), // provider permanently revoked the refresh token
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.error_kind)?;
        if let Some(ref src) = self.source {
            write!(f, ": {src}")?;
        }
        Ok(())
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn StdError + 'static))
    }
}

// This is where we translate errors from the `entity_api`` layer to the `domain`` layer.
impl From<EntityApiError> for Error {
    fn from(err: EntityApiError) -> Self {
        let entity_error_kind = match &err.error_kind {
            // Value-range violation → 422 `validation_error` (distinct from
            // `ValidationError` → 409, which is for entity-state conflicts
            // like goal-limit caps). Returned directly from the match so the
            // outer struct construction below is reserved for `Internal`
            // mappings only.
            EntityApiErrorKind::OutOfRange(out) => {
                let message = out.to_string();
                return Error {
                    source: Some(Box::new(err)),
                    error_kind: DomainErrorKind::Validation(message),
                };
            }
            EntityApiErrorKind::TopicReorderMismatch => {
                return Error {
                    source: Some(Box::new(err)),
                    error_kind: DomainErrorKind::Validation(
                        "Reorder id set does not match the coaching session's current topics."
                            .to_string(),
                    ),
                };
            }
            EntityApiErrorKind::RecordNotFound => EntityErrorKind::NotFound,
            EntityApiErrorKind::InvalidQueryTerm => EntityErrorKind::Invalid,
            EntityApiErrorKind::RecordUnauthenticated => EntityErrorKind::Unauthenticated,
            EntityApiErrorKind::ValidationError { message, details } => EntityErrorKind::Conflict {
                message: message.clone(),
                details: details.clone(),
            },
            EntityApiErrorKind::CannotLinkCompletedGoal => EntityErrorKind::CannotLinkCompletedGoal,
            EntityApiErrorKind::GoalAlreadyLinkedToSession => {
                EntityErrorKind::GoalAlreadyLinkedToSession
            }
            EntityApiErrorKind::SystemError => EntityErrorKind::ServiceUnavailable,
            _ => EntityErrorKind::Other("EntityErrorKind".to_string()),
        };

        Error {
            source: Some(Box::new(err)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(entity_error_kind)),
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        // Errors that result from issues building the reqwest::Client instance. This
        // type of error will occur prior to any network calls being made.
        if err.is_builder() {
            Error {
                source: Some(Box::new(err)),
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Failed to build reqwest client".to_string(),
                )),
            }
        // Errors that result from issues with the network call itself.
        } else {
            Error {
                source: Some(Box::new(err)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        }
    }
}

impl From<jsonwebtoken::errors::Error> for Error {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        Error {
            source: Some(Box::new(err)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "JWT encoding related error".to_string(),
            )),
        }
    }
}

impl From<meeting_ai::Error> for Error {
    fn from(err: meeting_ai::Error) -> Self {
        let error_kind = match &err {
            meeting_ai::Error::Network(_) | meeting_ai::Error::Timeout(_) => {
                DomainErrorKind::External(ExternalErrorKind::Network)
            }
            meeting_ai::Error::Configuration(_) => {
                DomainErrorKind::Internal(InternalErrorKind::Config)
            }
            meeting_ai::Error::NotFound(_) => {
                DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound))
            }
            other => DomainErrorKind::External(ExternalErrorKind::Other(other.to_string())),
        };
        Error {
            source: Some(Box::new(err)),
            error_kind,
        }
    }
}

impl From<MeetingAuthError> for Error {
    fn from(err: MeetingAuthError) -> Self {
        let error_kind = match &err.error_kind {
            MeetingAuthErrorKind::Http(_) => DomainErrorKind::External(ExternalErrorKind::Network),
            MeetingAuthErrorKind::OAuth(OAuthErrorKind::TokenRevoked) => DomainErrorKind::External(
                ExternalErrorKind::Other("oauth_token_revoked".to_string()),
            ),
            MeetingAuthErrorKind::OAuth(_) => {
                DomainErrorKind::External(ExternalErrorKind::Other("OAuth error".to_string()))
            }
            MeetingAuthErrorKind::Storage(_) | MeetingAuthErrorKind::Token(_) => {
                DomainErrorKind::Internal(InternalErrorKind::Other(err.to_string()))
            }
            MeetingAuthErrorKind::ApiKey(_)
            | MeetingAuthErrorKind::Credential(_)
            | MeetingAuthErrorKind::Webhook(_) => {
                DomainErrorKind::Internal(InternalErrorKind::Other(err.to_string()))
            }
        };
        Error {
            source: Some(Box::new(err)),
            error_kind,
        }
    }
}
