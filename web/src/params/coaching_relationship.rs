//! Parameters for coaching relationship endpoints.

use domain::ai_privacy_level::AiPrivacyLevel;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Parameters for updating a coaching relationship.
///
/// Different fields are accessible depending on the user's role:
/// - **Coach**: Can update `meeting_url` and `coach_ai_privacy_level`
/// - **Coachee**: Can only update `coachee_ai_privacy_level`
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct UpdateParams {
    /// Google Meet URL for this coaching relationship (coach only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_url: Option<String>,
    /// AI privacy level set by the coach (coach only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coach_ai_privacy_level: Option<AiPrivacyLevel>,
    /// AI privacy level set by the coachee (coachee only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coachee_ai_privacy_level: Option<AiPrivacyLevel>,
}
