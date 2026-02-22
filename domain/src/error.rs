//! Error types for the `domain` layer.
use entity_api::error::{EntityApiErrorKind, Error as EntityApiError};
use meeting_auth::error::{Error as MeetingAuthError, ErrorKind as MeetingAuthErrorKind};
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
    DbTransaction,
    Other(String),
}

/// Enum representing the various kinds of external errors that can occur in the `domain`` layer.
#[derive(Debug, PartialEq)]
pub enum ExternalErrorKind {
    Network,
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Domain Error: {self:?}")
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
        let entity_error_kind = match err.error_kind {
            EntityApiErrorKind::RecordNotFound => EntityErrorKind::NotFound,
            EntityApiErrorKind::InvalidQueryTerm => EntityErrorKind::Invalid,
            EntityApiErrorKind::RecordUnauthenticated => EntityErrorKind::Unauthenticated,
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

impl From<MeetingAuthError> for Error {
    fn from(err: MeetingAuthError) -> Self {
        let error_kind = match &err.error_kind {
            MeetingAuthErrorKind::Http(_) => DomainErrorKind::External(ExternalErrorKind::Network),
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
