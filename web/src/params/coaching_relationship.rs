//! Parameters for coaching relationship endpoints.

use domain::ai_privacy_level::AiPrivacyLevel;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Parameters for updating a coaching relationship
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateParams {
    /// Google Meet URL for this coaching relationship
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meeting_url: Option<String>,
    /// AI privacy level for this coaching relationship
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_privacy_level: Option<AiPrivacyLevel>,
}
