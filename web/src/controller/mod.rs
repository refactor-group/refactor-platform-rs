use serde::Serialize;

pub(crate) mod action_controller;
pub(crate) mod agreement_controller;
pub(crate) mod coaching_session_controller;
pub(crate) mod jwt_controller;
pub(crate) mod note_controller;
pub(crate) mod organization;
pub(crate) mod organization_controller;
pub(crate) mod overarching_goal_controller;
pub(crate) mod user_controller;
pub(crate) mod user_session_controller;

#[derive(Debug, Serialize)]
struct ApiResponse<T: Serialize> {
    // Eventually we can add meta, errors, etc.
    status_code: u16,
    data: T,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn new(status_code: u16, data: T) -> Self {
        Self { status_code, data }
    }
}
