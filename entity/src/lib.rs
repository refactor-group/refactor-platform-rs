use uuid::Uuid;

pub mod prelude;

// Core entities
pub mod actions;
pub mod actions_users;
pub mod agreements;
pub mod coachees;
pub mod coaches;
pub mod coaching_relationships;
pub mod coaching_sessions;
pub mod coaching_sessions_goals;
pub mod goals;
pub mod jwts;
pub mod links;
pub mod magic_link_tokens;
pub mod meeting_recording;
pub mod notes;
pub mod oauth_connections;
pub mod organizations;
pub mod provider;
pub mod roles;
pub mod status;
pub mod transcript_segment;
pub mod transcription;
pub mod user_invite_status;
pub mod user_roles;
pub mod users;

/// A type alias that represents any Entity's internal id field data type.
/// Aliased so that it's easy to change the underlying type if necessary.
pub type Id = Uuid;
