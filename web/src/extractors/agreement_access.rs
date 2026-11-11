use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use domain::{agreement as AgreementApi, agreements};
use log::*;

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser, coaching_session_access::authorize_session,
        not_found, parse_path_id_from_parts, RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user is a participant of the relationship that owns the `:id`
/// agreement, by way of the agreement's coaching session.
///
/// An unknown agreement is a 404; an unknown session is a 404 and a non-participant a 403, as
/// for [`authorize_session`]. On success, yields the agreement.
pub(crate) struct AgreementAccess {
    pub agreement: agreements::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for AgreementAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, state).await?;

        let agreement_id = parse_path_id_from_parts(parts, "id").await?;

        let agreement = AgreementApi::find_by_id(app_state.db_conn_ref(), agreement_id)
            .await
            .map_err(|e| {
                warn!("Authorization error finding agreement {agreement_id}: {e:?}");
                not_found()
            })?;

        authorize_session(&app_state, &user, agreement.coaching_session_id).await?;

        Ok(AgreementAccess { agreement })
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "agreement_access_tests.rs"]
mod tests;
