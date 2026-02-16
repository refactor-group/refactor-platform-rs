use uuid::Uuid;

pub mod prelude;

// Core entities
pub mod actions;
pub mod agreements;
pub mod coachees;
pub mod coaches;
pub mod coaching_relationships;
pub mod coaching_sessions;
pub mod jwts;
pub mod notes;
pub mod oauth_connections;
pub mod organizations;
pub mod overarching_goals;
pub mod provider;
pub mod roles;
pub mod status;
pub mod user_roles;
pub mod users;

// AI Meeting Integration entities
pub mod ai_privacy_level;
pub mod ai_suggested_items;
pub mod ai_suggestion;
pub mod meeting_recording_status;
pub mod meeting_recordings;
pub mod sentiment;
pub mod transcript_segments;
pub mod transcription_status;
pub mod transcriptions;
pub mod user_integrations;

/// A type alias that represents any Entity's internal id field data type.
/// Aliased so that it's easy to change the underlying type if necessary.
pub type Id = Uuid;
