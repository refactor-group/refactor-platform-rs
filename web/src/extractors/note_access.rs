use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use domain::{note as NoteApi, notes};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser, coaching_session_access::authorize_session,
        lookup_rejection, not_found, parse_path_id_from_parts, RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user is a participant of the relationship that owns the `:id`
/// note, by way of the note's coaching session.
///
/// An unknown note is a 404; an unknown session is a 404 and a non-participant a 403, as for
/// [`authorize_session`]. On success, yields the note.
pub(crate) struct NoteAccess {
    pub note: notes::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for NoteAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, state).await?;

        let note_id = parse_path_id_from_parts(parts, "id").await?;

        let note = NoteApi::find_by_id(app_state.db_conn_ref(), note_id)
            .await
            .map_err(|e| lookup_rejection(e.into()))?
            .ok_or_else(not_found)?;

        authorize_session(&app_state, &user, note.coaching_session_id).await?;

        Ok(NoteAccess { note })
    }
}
