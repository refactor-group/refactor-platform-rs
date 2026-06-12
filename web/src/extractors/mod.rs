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

use axum::{
    extract::{FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
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

/// Resolves the request's `Path` map then parses `key` as a UUID, mapping any
/// failure to a 400. `Path` reads from request extensions, so the state is unused.
pub(crate) async fn parse_path_id_from_parts(
    parts: &mut Parts,
    key: &str,
) -> Result<Id, RejectionType> {
    let Path(path_params) = Path::<HashMap<String, String>>::from_request_parts(parts, &())
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid path parameters".to_string(),
            )
        })?;
    parse_path_id(&path_params, key)
}

/// The uniform 404 rejection: an inaccessible resource looks identical to a missing one.
pub(crate) fn not_found() -> RejectionType {
    (StatusCode::NOT_FOUND, "NOT FOUND".to_string())
}
