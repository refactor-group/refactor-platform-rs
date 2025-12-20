//! Parameters for user integration endpoints.

use chrono::{DateTime, FixedOffset};
use domain::user_integrations;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Parameters for updating user integrations
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateIntegrationParams {
    /// Recall.ai API key (encrypted at rest)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall_ai_api_key: Option<String>,
    /// Recall.ai region (e.g., "us-west-2", "us-east-1", "eu-west-1")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall_ai_region: Option<String>,
    /// AssemblyAI API key (encrypted at rest)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assembly_ai_api_key: Option<String>,
}

/// Response for user integration status (without exposing keys)
#[derive(Debug, Serialize, ToSchema)]
pub struct IntegrationStatusResponse {
    /// Whether Google account is connected
    pub google_connected: bool,
    /// Connected Google email (if any)
    pub google_email: Option<String>,
    /// Whether Recall.ai is configured
    pub recall_ai_configured: bool,
    /// Recall.ai region
    pub recall_ai_region: Option<String>,
    /// When Recall.ai was last verified
    pub recall_ai_verified_at: Option<String>,
    /// Whether AssemblyAI is configured
    pub assembly_ai_configured: bool,
    /// When AssemblyAI was last verified
    pub assembly_ai_verified_at: Option<String>,
}

impl From<user_integrations::Model> for IntegrationStatusResponse {
    fn from(model: user_integrations::Model) -> Self {
        Self {
            google_connected: model.google_access_token.is_some(),
            google_email: model.google_email,
            recall_ai_configured: model.recall_ai_api_key.is_some(),
            recall_ai_region: model.recall_ai_region,
            recall_ai_verified_at: model
                .recall_ai_verified_at
                .map(|dt: DateTime<FixedOffset>| dt.to_rfc3339()),
            assembly_ai_configured: model.assembly_ai_api_key.is_some(),
            assembly_ai_verified_at: model
                .assembly_ai_verified_at
                .map(|dt: DateTime<FixedOffset>| dt.to_rfc3339()),
        }
    }
}

/// Response for API key verification
#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyApiKeyResponse {
    /// Whether the API key is valid
    pub valid: bool,
    /// Optional message
    pub message: Option<String>,
}
