//! Error types for entity API
use std::error::Error as StdError;
use std::fmt;

use serde::Serialize;

use sea_orm::error::DbErr;

use entity::duration::OutOfRange;

/// Errors while executing operations related to entities.
/// The intent is to categorize errors into two major types:
///  * Errors related to data. Ex DbError::RecordNotFound
///  * Errors related to interactions with the database itself. Ex DbError::Conn
#[derive(Debug, PartialEq)]
pub struct Error {
    // Underlying error emitted from seaORM internals
    pub source: Option<DbErr>,
    // Enum representing which category of error
    pub error_kind: EntityApiErrorKind,
}

#[derive(Debug, PartialEq, Serialize)]
pub enum EntityApiErrorKind {
    // Invalid search term
    InvalidQueryTerm,
    // Record not found
    RecordNotFound,
    // Record not updated
    RecordNotUpdated,
    // Record not authenticated
    RecordUnauthenticated,
    // Errors related to interactions with the database itself. Ex DbError::Conn
    SystemError,
    // Validation error with descriptive message and optional structured details
    ValidationError {
        message: String,
        details: Option<serde_json::Value>,
    },
    // Attempt to link a goal in a completed status (Completed or WontDo) to a coaching session.
    CannotLinkCompletedGoal,
    // Attempt to link a goal that is already linked to the same coaching session.
    GoalAlreadyLinkedToSession,
    // Range-bounded entity construction failed (e.g. `Duration` outside
    // 1..=480). Maps to 422 in domain (distinct from `ValidationError`,
    // which is for 409 state conflicts like cap violations).
    OutOfRange(OutOfRange),
    // A text field exceeded its maximum length. Maps to 422 in domain (a
    // value-validation failure, distinct from `ValidationError` → 409 state
    // conflicts). `max`/`actual` are character counts, matching the column bound.
    TitleTooLong {
        max: usize,
        actual: usize,
    },
    // Other errors
    Other(String),
}

impl From<OutOfRange> for Error {
    fn from(err: OutOfRange) -> Self {
        Error {
            source: None,
            error_kind: EntityApiErrorKind::OutOfRange(err),
        }
    }
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

impl StdError for Error {}

impl From<DbErr> for Error {
    fn from(err: DbErr) -> Self {
        match err {
            DbErr::RecordNotFound(_) => Error {
                source: Some(err),
                error_kind: EntityApiErrorKind::RecordNotFound,
            },
            DbErr::RecordNotUpdated => Error {
                source: Some(err),
                error_kind: EntityApiErrorKind::RecordNotUpdated,
            },
            DbErr::ConnectionAcquire(_) => Error {
                source: Some(err),
                error_kind: EntityApiErrorKind::SystemError,
            },
            DbErr::Conn(_) => Error {
                source: Some(err),
                error_kind: EntityApiErrorKind::SystemError,
            },
            DbErr::Exec(_) => Error {
                source: Some(err),
                error_kind: EntityApiErrorKind::SystemError,
            },
            _ => Error {
                source: Some(err),
                error_kind: EntityApiErrorKind::SystemError,
            },
        }
    }
}
