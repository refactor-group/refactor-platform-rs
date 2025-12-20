use serde::Serialize;
pub(crate) mod action_controller;
pub(crate) mod agreement_controller;
pub(crate) mod coaching_relationship_controller;
pub(crate) mod coaching_session_controller;
pub(crate) mod health_check_controller;
pub(crate) mod integration_controller;
pub(crate) mod jwt_controller;
pub(crate) mod note_controller;
pub(crate) mod oauth_controller;
pub(crate) mod organization;
pub(crate) mod organization_controller;
pub(crate) mod overarching_goal_controller;
pub(crate) mod user;
pub(crate) mod user_controller;
pub(crate) mod user_session_controller;

#[derive(Debug, Serialize)]
struct ApiResponse<T: Serialize> {
    status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn new(status_code: u16, data: T) -> Self {
        Self {
            status_code,
            data: Some(data),
        }
    }

    pub fn no_content(status_code: u16) -> ApiResponse<()> {
        ApiResponse {
            status_code,
            data: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use serde_json::json;

    #[tokio::test]
    async fn test_serialize_api_response_with_some() {
        let response = ApiResponse {
            status_code: StatusCode::OK.into(),
            data: Some(23),
        };
        let serialized = serde_json::to_string(&response).unwrap();

        // Serializing and then deserializing because the string output from serde_json::to_string is
        // non-deterministic as far as the order of the JSON keys. This ensures the test won't be flaky
        let deserialized_value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        let deserialized_expected_value: serde_json::Value =
            json!({"data": 23, "status_code": 200});
        assert_eq!(deserialized_value, deserialized_expected_value);
    }

    #[tokio::test]
    async fn test_serialize_api_response_with_none() {
        let response = ApiResponse::<()>::no_content(StatusCode::NO_CONTENT.into());
        // No need to deserialize here because there's only one key
        let serialized = serde_json::to_string(&response).unwrap();
        assert_eq!(serialized, json!({"status_code": 204}).to_string());
    }
}
