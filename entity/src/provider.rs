use crate::meeting_provider;
use crate::pipeline_provider;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Rust-only wrapper enum for any provider in the system.
/// Not backed by a DB column — use `meeting_provider::Provider` or
/// `pipeline_provider::Provider` for DB fields.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize, ToSchema)]
#[serde(tag = "type", content = "provider", rename_all = "snake_case")]
pub enum Provider {
    Meeting(meeting_provider::Provider),
    Pipeline(pipeline_provider::Provider),
}

impl From<meeting_provider::Provider> for Provider {
    fn from(p: meeting_provider::Provider) -> Self {
        Provider::Meeting(p)
    }
}

impl From<pipeline_provider::Provider> for Provider {
    fn from(p: pipeline_provider::Provider) -> Self {
        Provider::Pipeline(p)
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Meeting(p) => write!(f, "{}", p),
            Provider::Pipeline(p) => write!(f, "{}", p),
        }
    }
}
