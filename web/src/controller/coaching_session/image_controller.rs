use std::time::Duration;

use axum::extract::{Multipart, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION, VARY};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::coaching_session_image::{
    self as CoachingSessionImageApi, ImageRejection, StoreImageParams,
};
use log::*;
use service::config::ApiVersion;

use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, coaching_session_access::CoachingSessionAccess,
    coaching_session_image_access::CoachingSessionImageAccess,
    compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};

/// Upload one image pasted into a coaching session's notes
///
/// The bytes are sniffed server-side; the declared content type is never trusted.
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/images",
    params(
        ApiVersion,
        ("coaching_session_id" = domain::Id, Path, description = "Coaching session id"),
    ),
    request_body(content = String, description = "multipart/form-data with a `file` part", content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Image stored", body = domain::coaching_session_images::Model),
        (status = 400, description = "No `file` part, or an empty one"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller is not a participant in the coaching session"),
        (status = 413, description = "Bytes exceed the configured image size cap"),
        (status = 415, description = "Sniffed type is not an allowed image type"),
        (status = 503, description = "Object storage is not configured"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    multipart: Multipart,
) -> Result<impl IntoResponse, Error> {
    // Checked before the body is read so an unconfigured deployment fails fast.
    let Some(store) = app_state.object_store.clone() else {
        return Ok(storage_unavailable());
    };

    let bytes = match file_bytes(multipart).await {
        Ok(bytes) => bytes,
        Err(response) => return Ok(response),
    };

    debug!("POST note image for session {}", session.id);

    let inspected = match CoachingSessionImageApi::inspect_image(
        &bytes,
        app_state.config.coaching_session_image_max_bytes(),
    ) {
        Ok(inspected) => inspected,
        Err(rejection) => return Ok(rejected(rejection)),
    };

    let image = CoachingSessionImageApi::create(
        app_state.db_conn_ref(),
        store.as_ref(),
        StoreImageParams {
            coaching_session_id: session.id,
            uploaded_by_id: user.id,
            bytes,
            inspected: &inspected,
        },
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), image)).into_response())
}

/// Fetch one note image, as a redirect to a presigned URL or as the bytes themselves
///
/// DELIBERATELY NO `CompareApiVersion`: this URL is loaded by a browser `<img>` tag, which
/// cannot send the `X-Version` header. Adding that extractor here to make the API uniform
/// would silently break every image in the product.
#[utoipa::path(
    get,
    path = "/coaching_session_images/{image_id}",
    params(
        ("image_id" = domain::Id, Path, description = "Note image id"),
    ),
    responses(
        (status = 200, description = "The image bytes, when the storage backend cannot presign"),
        (status = 302, description = "Redirect to a presigned URL for the image"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller is not a participant in the image's coaching session"),
        (status = 404, description = "No such image"),
        (status = 503, description = "Object storage is not configured"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn read(
    CoachingSessionImageAccess(image): CoachingSessionImageAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let Some(store) = app_state.object_store.clone() else {
        return Ok(storage_unavailable());
    };

    let presign_ttl_seconds = app_state
        .config
        .coaching_session_image_presign_ttl_seconds();

    let mut response =
        match store.presigned_get(&image.storage_key, Duration::from_secs(presign_ttl_seconds))? {
            Some(url) => redirect_to(&url)?,
            None => stream(store.get(&image.storage_key).await?)?,
        };

    // Both branches: the bytes belong to one relationship, so a shared cache must key on
    // the session cookie, and the entry must expire before the signature it may carry.
    let headers = response.headers_mut();
    headers.insert(
        CACHE_CONTROL,
        header_value(&format!(
            "private, max-age={}",
            cache_max_age(presign_ttl_seconds)
        ))?,
    );
    headers.insert(VARY, HeaderValue::from_static("Cookie"));

    Ok(response)
}

/// Cache lifetime for a note image response, derived from the presign TTL rather than
/// configured beside it: two independent numbers would eventually drift and let a cached
/// redirect outlive its own signature. Two thirds leaves room for clock skew and the trip
/// back to the client.
fn cache_max_age(presign_ttl_seconds: u64) -> u64 {
    presign_ttl_seconds / 3 * 2
}

/// A 302 to the presigned URL. The browser fetches the bytes from storage directly.
fn redirect_to(url: &str) -> Result<Response, Error> {
    Ok((StatusCode::FOUND, [(LOCATION, header_value(url)?)]).into_response())
}

/// The bytes themselves, served with the content type recorded on the stored object.
/// The fallback for backends that cannot presign.
fn stream(stored: domain::gateway::object_storage::StoredObject) -> Result<Response, Error> {
    Ok((
        StatusCode::OK,
        [(CONTENT_TYPE, header_value(&stored.content_type)?)],
        stored.bytes,
    )
        .into_response())
}

/// Bytes of the `file` part. `Err` carries the response to return: a multipart failure
/// reports its own status (413 once the route's body limit is hit), while an absent or
/// empty part is the caller sending us nothing to store.
async fn file_bytes(mut multipart: Multipart) -> Result<Vec<u8>, Response> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(IntoResponse::into_response)?
    {
        if field.name() == Some("file") {
            let bytes = field.bytes().await.map_err(IntoResponse::into_response)?;

            return (!bytes.is_empty())
                .then(|| bytes.to_vec())
                .ok_or_else(missing_file_part);
        }
    }

    Err(missing_file_part())
}

fn missing_file_part() -> Response {
    (StatusCode::BAD_REQUEST, "MISSING FILE PART").into_response()
}

/// Rejected bytes, mapped to the two statuses the frontend branches on. Nowhere else in
/// the API produces either, so collapsing them would make the distinction unrecoverable.
fn rejected(rejection: ImageRejection) -> Response {
    match rejection {
        ImageRejection::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD TOO LARGE"),
        ImageRejection::UnsupportedType => {
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED MEDIA TYPE")
        }
    }
    .into_response()
}

/// Object storage is a deployment concern, so its absence is 503 rather than a 500 that
/// would read as a bug.
fn storage_unavailable() -> Response {
    warn!("Note image request refused: object storage is not configured");
    (StatusCode::SERVICE_UNAVAILABLE, "SERVICE UNAVAILABLE").into_response()
}

/// A header value we built ourselves, or a 500 if storage handed back something unsendable.
fn header_value(value: &str) -> Result<HeaderValue, Error> {
    HeaderValue::from_str(value).map_err(|_| Error::Web(WebErrorKind::Other))
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "image_controller_tests.rs"]
mod tests;
