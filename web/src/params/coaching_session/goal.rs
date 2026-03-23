use serde::Deserialize;
use utoipa::ToSchema;

use domain::Id;

/// Request body for linking an existing goal to a coaching session.
/// The `coaching_session_id` comes from the URL path parameter.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct LinkParams {
    pub(crate) goal_id: Id,
}
