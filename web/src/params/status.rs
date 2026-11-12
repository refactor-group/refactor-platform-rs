//! Query parameters for the action and goal status routes.

use domain::status::Status;
use serde::Deserialize;

use crate::Error;

/// Query parameters for setting a status.
#[derive(Debug, Deserialize)]
pub(crate) struct UpdateParams {
    /// The status's snake_case value, e.g. `completed`.
    pub(crate) value: String,
}

impl UpdateParams {
    /// The requested status; an unknown value is a 422.
    pub(crate) fn status(&self) -> Result<Status, Error> {
        Status::try_from(self.value.as_str())
            .map_err(|_| Error::validation(format!("'{}' is not a known status.", self.value)))
    }
}
