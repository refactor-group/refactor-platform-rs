use entity_api::error::{EntityApiErrorKind, Error as EntityApiError};
use std::error::Error as StdError;
use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub source: Option<Box<dyn StdError + Send + Sync>>,
    pub error_kind: DomainErrorKind,
}
#[derive(Debug, PartialEq)]
pub enum DomainErrorKind {
    Internal(InternalErrorKind),
    External(ExternalErrorKind),
}
#[derive(Debug, PartialEq)]
pub enum InternalErrorKind {
    Entity(EntityErrorKind),
    Other,
}

#[derive(Debug, PartialEq)]
pub enum EntityErrorKind {
    NotFound,
    Invalid,
    Other,
}

#[derive(Debug, PartialEq)]
pub enum ExternalErrorKind {
    Network,
    Other,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Domain Error: {:?}", self)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn StdError + 'static))
    }
}

impl From<EntityApiError> for Error {
    fn from(err: EntityApiError) -> Self {
        let entity_error_kind = match err.error_kind {
            EntityApiErrorKind::RecordNotFound => EntityErrorKind::NotFound,
            EntityApiErrorKind::InvalidQueryTerm => EntityErrorKind::Invalid,
            _ => EntityErrorKind::Other,
        };

        Error {
            source: Some(Box::new(err)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(entity_error_kind)),
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        if err.is_builder() {
            Error {
                source: Some(Box::new(err)),
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
            }
        } else {
            Error {
                source: Some(Box::new(err)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        }
    }
}
