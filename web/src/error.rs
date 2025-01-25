use std::error::Error as StdError;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use entity_api::error::EntityApiErrorKind;
use entity_api::error::Error as EntityApiError;

extern crate log;
use log::*;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error(EntityApiError);

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
            EntityApiErrorKind::InvalidQueryTerm => {
                error!(
                    "Error: {:#?}, mapping to UNPROCESSABLE_ENTITY (reason: {})",
                    self,
                    self.0
                        .inner
                        .as_ref()
                        .map_or_else(|| "unspecified".to_string(), |err| err.to_string())
                );

                (StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE ENTITY").into_response()
            }
            EntityApiErrorKind::SystemError => {
                error!(
                    "Error: {:#?}, mapping to INTERNAL_SERVER_ERROR (reason: {})",
                    self,
                    self.0
                        .inner
                        .as_ref()
                        .map_or_else(|| "unspecified".to_string(), |err| err.to_string())
                );

                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
            }
            EntityApiErrorKind::RecordNotFound => {
                error!("Error: {:#?}, mapping to NO_CONTENT", self);

                (StatusCode::NOT_FOUND, "NOT FOUND").into_response()
            }
            EntityApiErrorKind::RecordNotUpdated => {
                error!(
                    "Error: {:#?}, mapping to UNPROCESSABLE_ENTITY (reason: {})",
                    self,
                    self.0
                        .inner
                        .as_ref()
                        .map_or_else(|| "unspecified".to_string(), |err| err.to_string())
                );

                (StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE ENTITY").into_response()
            }
            EntityApiErrorKind::RecordUnauthenticated => {
                error!("Error: {:#?}, mapping to UNAUTHORIZED", self);

                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
        }
    }
}

impl<E> From<E> for Error
where
    E: Into<EntityApiError>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
