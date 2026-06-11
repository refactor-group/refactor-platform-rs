pub(crate) mod authenticated_user;
pub(crate) mod coaching_relationship_access;
pub(crate) mod coaching_session_access;
pub(crate) mod coaching_session_topic_access;
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
use domain::Id;
use std::collections::HashMap;

type RejectionType = (StatusCode, String);

/// Parses a UUID path segment, mapping a missing or malformed value to a 400.
/// Shared across extractors so `:topic_id`/`:user_id`/etc. parsing stays uniform.
pub(crate) fn parse_path_id(
    params: &HashMap<String, String>,
    key: &str,
) -> Result<Id, RejectionType> {
    params
        .get(key)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, format!("Missing {key} in path")))?
        .parse::<Id>()
        .map_err(|_| (StatusCode::BAD_REQUEST, format!("Invalid {key}")))
}
