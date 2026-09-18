pub(crate) mod coaching_relationship;
pub(crate) mod coaching_relationship_controller;
pub(crate) mod user;
pub(crate) mod user_controller;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_controller_tests.rs"]
mod user_controller_tests;
