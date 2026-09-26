use std::borrow::Cow;

use axum::extract::{Path, State};
use axum::http::header::{ACCEPT, CONTENT_DISPOSITION, CONTENT_TYPE, VARY};
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

use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::{
    coaching_session_access::CoachingSessionAccess, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};

/// The repeatable `speaker` query parameter limiting the plain-text transcript.
#[derive(Debug, Deserialize)]
pub(crate) struct SpeakerParams {
    #[serde(default)]
    speaker: Vec<SpeakerRole>,
}

/// The representation the caller's `Accept` header asked for.
#[derive(Clone, Copy)]
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
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
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
/// `speaker` narrows the plain-text file to the named participants; JSON validates it but
/// does not apply it. Browsers send `text/html, ..., */*`, which selects JSON, so a plain
/// link downloads JSON; fetch with an explicit `Accept: text/plain` and build the file.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcriptions/{transcription_id}",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("transcription_id" = Uuid, Path, description = "Transcription id"),
        ("speaker" = Option<Vec<SpeakerRole>>, Query, explode, description = "Limit the plain-text transcript to these participants. Repeat the parameter for both. Omitted means every speaker. An invalid value is rejected with 400 for every representation; valid values are applied only to text/plain."),
    ),
    responses(
        (status = 200, description = "JSON metadata with resolved speakers, or the plain-text transcript file when `Accept: text/plain`", content(
            (domain::transcription::WithSpeakers = "application/json"),
            (String = "text/plain", example = json!("Coaching session transcript\nDate: 2026-09-21\nSpeakers: Jim Hodapp, Caleb Bourg\n\n[0:00] Jim Hodapp: Good morning.\n"))
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
        Error::Web(WebErrorKind::InvalidSpeaker(speaker_values(
            uri.query().unwrap_or_default(),
        )))
    })?;

    debug!(
        "GET transcription {transcription_id} for session {}",
        session.id
    );

    let mut response = match negotiate(&headers) {
        None => (StatusCode::NOT_ACCEPTABLE, "NOT ACCEPTABLE").into_response(),
        Some(Representation::Json) => {
            let body = TranscriptionApi::read_with_speakers(
                app_state.db_conn_ref(),
                &session,
                transcription_id,
            )
            .await?;

            Json(ApiResponse::new(StatusCode::OK.into(), body)).into_response()
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

            (
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
                .into_response()
        }
    };

    // One URL, two representations: caches must key on Accept.
    response
        .headers_mut()
        .insert(VARY, HeaderValue::from_static("accept"));
    Ok(response)
}

/// The decoded `speaker` values in a query string, bounded, for the 400 message.
fn speaker_values(query: &str) -> String {
    query
        .split('&')
        .filter_map(|pair| pair.strip_prefix("speaker="))
        .map(|value| {
            let spaced = value.replace('+', " ");
            urlencoding::decode(&spaced)
                .map(Cow::into_owned)
                .unwrap_or(spaced)
        })
        .collect::<Vec<_>>()
        .join(", ")
        .chars()
        .take(80)
        .collect()
}

/// Picks the representation from `Accept`, honouring quality values.
///
/// Absent means JSON. Otherwise the supported media type with the highest `q` wins,
/// earlier entries first on a tie, `q=0` excludes a type, and a malformed `q` drops its entry. `*/*`, `application/json`,
/// `application/*` select JSON; `text/plain`, `text/*` select the transcript file.
/// Nothing acceptable means 406.
fn negotiate(headers: &HeaderMap) -> Option<Representation> {
    let mut values = headers.get_all(ACCEPT).iter().peekable();
    if values.peek().is_none() {
        return Some(Representation::Json);
    }

    values
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|entry| {
            let mut parts = entry.split(';');
            let media = parts.next()?.trim().to_ascii_lowercase();
            // `q` is case-insensitive and must be 0..=1; a malformed one is no valid request.
            let quality = match parts
                .filter_map(|param| param.split_once('='))
                .find(|(name, _)| name.trim().eq_ignore_ascii_case("q"))
                .map(|(_, value)| value.trim().parse::<f32>().ok())
            {
                Some(Some(q)) if (0.0..=1.0).contains(&q) => q,
                Some(_) => return None,
                None => 1.0,
            };
            let representation = match media.as_str() {
                "*/*" | "application/json" | "application/*" => Representation::Json,
                "text/plain" | "text/*" => Representation::PlainText,
                _ => return None,
            };
            (quality > 0.0).then_some((quality, representation))
        })
        .fold(
            None,
            |best: Option<(f32, Representation)>, candidate| match best {
                Some((quality, _)) if quality >= candidate.0 => best,
                _ => Some(candidate),
            },
        )
        .map(|(_, representation)| representation)
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "transcription_controller_tests.rs"]
mod tests;
