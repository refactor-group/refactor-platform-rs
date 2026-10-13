use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use domain::transcript_suggestion;

use crate::{
    extractors::{
        coaching_session_access::CoachingSessionAccess, not_found, parse_path_id_from_parts,
        RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user is a participant of the path session AND that the
/// `:suggestion_id` suggestion belongs to that session.
///
/// Composes `CoachingSessionAccess`, then loads the suggestion and confirms it belongs to the
/// path session. Any failure collapses to 404 so a suggestion in an inaccessible session is
/// never revealed. On success, yields the suggestion.
pub(crate) struct TranscriptSuggestionAccess {
    pub suggestion: transcript_suggestion::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for TranscriptSuggestionAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let CoachingSessionAccess(session) =
            CoachingSessionAccess::from_request_parts(parts, state)
                .await
                .map_err(|_| not_found())?;

        let suggestion_id = parse_path_id_from_parts(parts, "suggestion_id")
            .await
            .map_err(|_| not_found())?;

        let suggestion = transcript_suggestion::find_by_id(app_state.db_conn_ref(), suggestion_id)
            .await
            .map_err(|_| not_found())?;

        if suggestion.coaching_session_id != session.id {
            return Err(not_found());
        }

        Ok(TranscriptSuggestionAccess { suggestion })
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "transcript_suggestion_access_tests.rs"]
mod tests;
