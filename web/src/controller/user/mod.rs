pub(crate) mod action_controller;
pub(crate) mod coaching_relationships_controller;
pub(crate) mod coaching_session_controller;
pub(crate) mod goal_controller;
pub(crate) mod organization_controller;
pub(crate) mod password_controller;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_session_controller_tests.rs"]
mod coaching_session_controller_tests;
