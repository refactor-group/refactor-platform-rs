use uuid::Uuid;

pub mod prelude;

pub mod actions;
pub mod agreements;
pub mod coachees;
pub mod coaches;
pub mod coaching_relationships;
pub mod coaching_sessions;
pub mod jwts;
pub mod notes;
pub mod organizations;
pub mod overarching_goals;
pub mod status;
pub mod users;

/// A type alias that represents any Entity's internal id field data type.
/// Aliased so that it's easy to change the underlying type if necessary.
pub type Id = Uuid;
