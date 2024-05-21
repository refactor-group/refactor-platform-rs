use uuid::Uuid;

pub mod prelude;

pub mod actions;
pub mod agreements;
pub mod coaching_relationships;
pub mod coaching_sessions;
pub mod notes;
pub mod organizations;
pub mod overarching_goals;
pub mod users;

/// A type alias that represents any Entity's internal id field data type
pub type Id = Uuid;
