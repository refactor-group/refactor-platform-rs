pub(crate) mod authenticated_user;
pub(crate) mod coaching_relationship_access;
pub(crate) mod coaching_session_access;
pub(crate) mod coaching_session_series_access;
pub(crate) mod compare_api_version;
pub(crate) mod organization_member_access;
pub(crate) mod organization_user_access;
pub(crate) mod svix_signature;

#[cfg(test)]
#[cfg(feature = "mock")]
mod session_renewal_tests;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "organization_user_access_tests.rs"]
mod organization_user_access_tests;

use axum::http::StatusCode;

type RejectionType = (StatusCode, String);
