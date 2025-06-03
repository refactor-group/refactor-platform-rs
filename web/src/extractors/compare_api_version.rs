use crate::extractors::RejectionType;
use crate::AppState;
use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{header::HeaderValue, request::Parts, StatusCode},
};
use log::*;
use service::config::ApiVersion;

pub struct CompareApiVersion(pub HeaderValue);

#[async_trait]
impl<S> FromRequestParts<S> for CompareApiVersion
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    // A custom Extractor that extracts and checks that the API version number
    // provided in the "X-Version" header is equal to the API version specified
    // in AppState.
    // If this Extractor fails any Handler methods that use it will not be called
    // successfully.
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        // Provided by the client in the HTTP header
        let version = get_x_version(parts)?;
        debug!("API version provided by client: {:?}", version);
        // Provided as part of the AppState environment configuration
        let api_version = HeaderValue::from_str(state.config.api_version())
            .ok()
            .unwrap_or_else(|| HeaderValue::from_static(ApiVersion::default_version()));
        debug!(
            "API version set in AppState.config.api_version: {:?}",
            api_version
        );

        Ok(is_current_api_version(version, api_version)?)
    }
}

fn get_x_version(parts: &mut Parts) -> Result<HeaderValue, RejectionType> {
    if let Some(version) = parts.headers.get(ApiVersion::field_name()) {
        Ok(version.clone())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            format!("`{}` header is missing", ApiVersion::field_name()),
        ))
    }
}

fn normalize_version(value: &HeaderValue) -> String {
    let s = value.to_str().unwrap_or_default().trim();
    s.trim_matches('"').to_string()
}

fn is_current_api_version(
    version: HeaderValue,
    api_version: HeaderValue,
) -> Result<CompareApiVersion, RejectionType> {
    let version_str = normalize_version(&version);
    let api_version_str = normalize_version(&api_version);
    debug!(
        "API version comparison {:?} == {:?}: {}",
        version_str,
        api_version_str,
        version_str == api_version_str
    );
    if version_str == api_version_str {
        Ok(CompareApiVersion(version))
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            format!(
                "`{}` header is not a valid API version",
                ApiVersion::field_name()
            ),
        ))
    }
}
