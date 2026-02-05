//! Error handling for the web layer.
//! Errors from lower layers are translated through `domain` to `web`
//! so that `web` can return appropriate HTTP status codes and messages to the client.
use std::error::Error as StdError;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use domain::error::{
    DomainErrorKind, EntityErrorKind, Error as DomainError, ExternalErrorKind, InternalErrorKind,
};

use log::*;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Domain(DomainError),
    Web(WebErrorKind),
}

#[derive(Debug)]
pub enum WebErrorKind {
    Input,
    Auth,
    Other,
}

impl StdError for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> core::result::Result<(), std::fmt::Error> {
        write!(fmt, "{self:?}")
    }
}

// List of possible StatusCode variants https://docs.rs/http/latest/http/status/struct.StatusCode.html#associatedconstant.UNPROCESSABLE_ENTITY
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Error::Domain(ref domain_error) => self.handle_domain_error(domain_error),
            Error::Web(ref web_error_kind) => self.handle_web_error(web_error_kind),
        }
    }
}

impl Error {
    fn handle_domain_error(&self, domain_error: &DomainError) -> Response {
        match domain_error.error_kind {
            DomainErrorKind::Internal(ref internal_error_kind) => {
                self.handle_internal_error(internal_error_kind)
            }
            DomainErrorKind::External(ref external_error_kind) => {
                self.handle_external_error(external_error_kind)
            }
        }
    }

    fn handle_internal_error(&self, internal_error_kind: &InternalErrorKind) -> Response {
        match internal_error_kind {
            InternalErrorKind::Entity(ref entity_error_kind) => {
                self.handle_entity_error(entity_error_kind)
            }
            InternalErrorKind::Config => {
                warn!(
                    "InternalErrorKind::Config: Responding with 500 Internal Server Error. Error: {self:?}"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
            }
            InternalErrorKind::Other(_description) => {
                warn!(
                    "InternalErrorKind::Other: Responding with 500 Internal Server Error. Error:: {self:?}"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
            }
        }
    }

    fn handle_entity_error(&self, entity_error_kind: &EntityErrorKind) -> Response {
        match entity_error_kind {
            EntityErrorKind::NotFound => {
                warn!("EntityErrorKind::NotFound: Responding with 404 Not Found. Error: {self:?}");
                (StatusCode::NOT_FOUND, "NOT FOUND").into_response()
            }
            EntityErrorKind::Unauthenticated => {
                warn!(
                    "EntityErrorKind::Unauthenticated: Responding with 401 Unauthorized. Error: {self:?}"
                );
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
            EntityErrorKind::DbTransaction => {
                warn!(
                    "EntityErrorKind::DbTransaction: Responding with 500 Internal Server Error. Error: {self:?}"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
            }
            EntityErrorKind::Invalid => {
                warn!(
                    "EntityErrorKind::Invalid: Responding with 422 Unprocessable Entity. Error: {self:?}"
                );
                (StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE ENTITY").into_response()
            }
            EntityErrorKind::ServiceUnavailable => {
                warn!(
                    "EntityErrorKind::ServiceUnavailable: Responding with 503 Service Unavailable. Error: {self:?}"
                );
                (StatusCode::SERVICE_UNAVAILABLE, "SERVICE UNAVAILABLE").into_response()
            }
            EntityErrorKind::Other(_description) => {
                warn!(
                    "EntityErrorKind::Other: Responding with 500 Internal Server Error. Error: {self:?}"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
            }
        }
    }

    fn handle_external_error(&self, external_error_kind: &ExternalErrorKind) -> Response {
        match external_error_kind {
            ExternalErrorKind::Network => {
                warn!(
                    "ExternalErrorKind::Network: Responding with 502 Bad Gateway. Error: {self:?}"
                );
                (StatusCode::BAD_GATEWAY, "BAD GATEWAY").into_response()
            }
            ExternalErrorKind::Other(_description) => {
                warn!(
                    "ExternalErrorKind::Other: Responding with 500 Internal Server Error. Error: {self:?}"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
            }
        }
    }

    fn handle_web_error(&self, web_error_kind: &WebErrorKind) -> Response {
        match web_error_kind {
            WebErrorKind::Input => {
                warn!("WebErrorKind::Input: Responding with 400 Bad Request. Error: {self:?}");
                (StatusCode::BAD_REQUEST, "BAD REQUEST").into_response()
            }
            WebErrorKind::Auth => {
                warn!("WebErrorKind::Auth: Responding with 401 Unauthorized. Error: {self:?}");
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
            WebErrorKind::Other => {
                warn!(
                    "WebErrorKind::Other: Responding with 500 Internal Server Error. Error: {self:?}"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
            }
        }
    }
}

impl<E> From<E> for Error
where
    E: Into<DomainError>,
{
    fn from(err: E) -> Self {
        Error::Domain(err.into())
    }
}
