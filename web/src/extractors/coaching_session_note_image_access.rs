use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};
use domain::{coaching_session, coaching_session_note_image, coaching_session_note_images};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser, not_found, parse_path_id_from_parts, RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user participates in the coaching session that owns the
/// `:image_id` note image.
///
/// The image id travels inside permanent CRDT note bytes and is therefore never a bearer
/// token: access is re-evaluated on every request against the session's current
/// relationship. A missing or unparseable id is a 400, an unknown image collapses to 404
/// so its existence is not revealed, and a non-participant gets 403. On success, yields
/// the image model so the handler needs no second query.
pub(crate) struct CoachingSessionNoteImageAccess(pub coaching_session_note_images::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionNoteImageAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let AuthenticatedUser(authenticated_user) =
            AuthenticatedUser::from_request_parts(parts, &app_state).await?;

        let image_id = parse_path_id_from_parts(parts, "image_id").await?;

        let image = coaching_session_note_image::find_by_id(app_state.db_conn_ref(), image_id)
            .await
            .map_err(|_| not_found())?;

        // The image itself carries no authorization; its session's relationship does.
        let (_session, coaching_relationship) =
            coaching_session::find_by_id_with_coaching_relationship(
                app_state.db_conn_ref(),
                image.coaching_session_id,
            )
            .await
            .map_err(|_| not_found())?;

        if !coaching_relationship.grants_access_to(&authenticated_user) {
            return Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()));
        }

        Ok(CoachingSessionNoteImageAccess(image))
    }
}
