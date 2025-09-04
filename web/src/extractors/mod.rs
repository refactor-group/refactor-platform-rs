pub(crate) mod authenticated_user;
pub(crate) mod compare_api_version;
pub(crate) mod read_only_session;

#[cfg(test)]
#[cfg(feature = "mock")]
mod session_renewal_tests;

use axum::http::StatusCode;

type RejectionType = (StatusCode, String);
