use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::{
    coaching_session_access::CoachingSessionAccess, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::header::{ACCEPT, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::{Query, QueryRejection};
use domain::transcript_export::SpeakerRole;
use domain::transcription as TranscriptionApi;
use domain::Id;
use log::*;
use serde::Deserialize;
use service::config::ApiVersion;

/// The repeatable `speaker` query parameter limiting the plain-text transcript.
#[derive(Debug, Deserialize)]
pub(crate) struct SpeakerParams {
    #[serde(default)]
    speaker: Vec<SpeakerRole>,
}

/// The representation the caller's `Accept` header asked for.
enum Representation {
    Json,
    PlainText,
}

/// Read the most recent transcription for a coaching session
///
/// A session can hold several transcriptions; this returns the latest by creation time.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcriptions",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Transcription metadata retrieved"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn read_latest(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let coaching_session_id = session.id;
    debug!("GET transcription for session {}", coaching_session_id);

    let transcription =
        TranscriptionApi::find_by_coaching_session(app_state.db_conn_ref(), coaching_session_id)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), transcription)))
}

/// Read one transcription as JSON metadata or download it as a plain-text file
///
/// `Accept: text/plain` serves the rendered transcript as a file attachment; any other
/// supported value, or no `Accept` at all, serves JSON metadata with the resolved speakers.
/// `speaker` narrows the plain-text file to the named participants and is ignored by JSON.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcriptions/{transcription_id}",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
        ("transcription_id" = Id, Path, description = "Transcription id"),
        ("speaker" = Option<Vec<SpeakerRole>>, Query, explode, description = "Limit the plain-text transcript to these participants. Repeat the parameter for both. Omitted means every speaker. Ignored for the JSON representation."),
    ),
    responses(
        (status = 200, description = "JSON metadata with resolved speakers, or the plain-text transcript file when `Accept: text/plain`", content(
            ("application/json" = domain::transcription::WithSpeakers),
            ("text/plain" = String, example = json!("Coaching session transcript\nDate: 2026-09-21\nSpeakers: Jim Hodapp, Caleb Bourg\n\n[0:00] Jim Hodapp: Good morning.\n"))
        )),
        (status = 400, description = "`invalid_speaker`: speaker is not `coach` or `coachee`"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller is not a participant in the coaching session"),
        (status = 404, description = "`transcription_not_found`: no such transcription under this session"),
        (status = 406, description = "Accept names no supported representation"),
        (status = 409, description = "`transcription_not_completed`: plain text requested before transcription completed"),
        (status = 422, description = "`speaker_not_identified`: a requested participant matched no speaker label"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
    Path((_coaching_session_id, transcription_id)): Path<(Id, Id)>,
    uri: Uri,
    headers: HeaderMap,
    speaker: Result<Query<SpeakerParams>, QueryRejection>,
) -> Result<Response, Error> {
    // An unparseable `speaker` is a 400 regardless of the representation asked for.
    let Query(SpeakerParams { speaker }) = speaker.map_err(|_| {
        Error::Web(WebErrorKind::InvalidSpeaker(
            uri.query().unwrap_or_default().to_owned(),
        ))
    })?;

    debug!(
        "GET transcription {transcription_id} for session {}",
        session.id
    );

    match negotiate(&headers) {
        None => Ok((StatusCode::NOT_ACCEPTABLE, "NOT ACCEPTABLE").into_response()),
        Some(Representation::Json) => {
            let body = TranscriptionApi::read_with_speakers(
                app_state.db_conn_ref(),
                &session,
                transcription_id,
            )
            .await?;

            Ok(Json(ApiResponse::new(StatusCode::OK.into(), body)).into_response())
        }
        Some(Representation::PlainText) => {
            let rendered = TranscriptionApi::export_plain_text(
                app_state.db_conn_ref(),
                &session,
                transcription_id,
                &speaker,
            )
            .await?;

            let disposition =
                HeaderValue::from_str(&format!("attachment; filename=\"{}\"", rendered.filename))
                    .map_err(|_| Error::Web(WebErrorKind::Other))?;

            Ok((
                StatusCode::OK,
                [
                    (
                        CONTENT_TYPE,
                        HeaderValue::from_static("text/plain; charset=utf-8"),
                    ),
                    (CONTENT_DISPOSITION, disposition),
                ],
                rendered.body,
            )
                .into_response())
        }
    }
}

/// Picks the representation from `Accept`, ignoring q-values.
///
/// Absent means JSON. Otherwise the first listed media type that is supported wins:
/// `*/*`, `application/json`, `application/*` select JSON; `text/plain`, `text/*`
/// select the transcript file. Nothing supported means 406.
fn negotiate(headers: &HeaderMap) -> Option<Representation> {
    let mut values = headers.get_all(ACCEPT).iter().peekable();
    if values.peek().is_none() {
        return Some(Representation::Json);
    }

    values
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|media| media.split(';').next())
        .map(|media| media.trim().to_ascii_lowercase())
        .find_map(|media| match media.as_str() {
            "*/*" | "application/json" | "application/*" => Some(Representation::Json),
            "text/plain" | "text/*" => Some(Representation::PlainText),
            _ => None,
        })
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "transcription_controller_tests.rs"]
mod tests;
