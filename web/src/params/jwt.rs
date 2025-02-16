use entity::Id;
use serde::Deserialize;
use utoipa::IntoParams;

/// Parameters required to generate a collaboration token
///
/// # Fields
///
/// * `coaching_session_id` - The ID of the coaching session for which the token is being generated
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct GenerateCollabTokenParams {
    pub(crate) coaching_session_id: Id,
}
