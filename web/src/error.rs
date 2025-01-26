use std::error::Error as StdError;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use domain::error::{
    DomainErrorKind, EntityErrorKind, Error as DomainError, ExternalErrorKind, InternalErrorKind,
};

extern crate log;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error(DomainError);

impl StdError for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> core::result::Result<(), std::fmt::Error> {
        write!(fmt, "{self:?}")
    }
}

// List of possible StatusCode variants https://docs.rs/http/latest/http/status/struct.StatusCode.html#associatedconstant.UNPROCESSABLE_ENTITY
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self.0.error_kind {
            DomainErrorKind::Internal(internal_error_kind) => match internal_error_kind {
                InternalErrorKind::Entity(entity_error_kind) => match entity_error_kind {
                    EntityErrorKind::NotFound => {
                        (StatusCode::NOT_FOUND, "NOT FOUND").into_response()
                    }
                    EntityErrorKind::Invalid => {
                        (StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE ENTITY").into_response()
                    }
                    EntityErrorKind::Other => {
                        (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
                    }
                },
                InternalErrorKind::Other => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
                }
            },
            DomainErrorKind::External(external_error_kind) => match external_error_kind {
                ExternalErrorKind::Network => {
                    (StatusCode::BAD_GATEWAY, "BAD GATEWAY").into_response()
                }
                ExternalErrorKind::Other => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
                }
            },
        }
    }
}

impl<E> From<E> for Error
where
    E: Into<DomainError>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
