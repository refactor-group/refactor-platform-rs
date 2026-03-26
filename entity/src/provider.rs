use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum, Default,
)]
#[serde(rename_all = "lowercase")]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "provider")]
pub enum Provider {
    #[sea_orm(string_value = "google")]
    #[default]
    Google,

    #[sea_orm(string_value = "zoom")]
    Zoom,
}

/// Describes meeting-space lifecycle behavior for a video-conferencing provider.
///
/// Some providers (e.g. Google Meet) create persistent spaces whose URL never expires,
/// making it safe — and desirable — to reuse the same link across sessions in a coaching
/// relationship. Other providers (e.g. Zoom) create time-bound meetings that expire, so
/// each session needs a fresh link.
pub trait MeetingProperties {
    /// Whether meeting URLs from this provider are persistent and can be reused
    /// across sessions within the same coaching relationship.
    fn has_persistent_meeting_urls(&self) -> bool;
}

impl MeetingProperties for Provider {
    fn has_persistent_meeting_urls(&self) -> bool {
        match self {
            Self::Google => true,
            Self::Zoom => false,
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Google => write!(f, "Google"),
            Self::Zoom => write!(f, "Zoom"),
        }
    }
}
