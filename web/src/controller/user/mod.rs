use domain::{users, Id};

use crate::error::WebErrorKind;
use crate::Error;

pub(crate) mod action_controller;
pub(crate) mod coaching_relationships_controller;
pub(crate) mod coaching_session_controller;
pub(crate) mod goal_controller;
pub(crate) mod organization_controller;
pub(crate) mod password_controller;

/// The guard's 403 unless `user_id` is the caller's own: these lists read only the caller.
fn ensure_self(user: &users::Model, user_id: Id) -> Result<(), Error> {
    (user.id == user_id)
        .then_some(())
        .ok_or(Error::Web(WebErrorKind::Forbidden))
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_session_controller_tests.rs"]
mod coaching_session_controller_tests;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "own_lists_tests.rs"]
mod own_lists_tests;
