use domain::{IntoUpdateMap, UpdateMap};
use sea_orm::Value;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct UpdateParams {
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub display_name: Option<String>,
    pub github_profile_url: Option<String>,
    pub timezone: Option<String>,
}
impl IntoUpdateMap for UpdateParams {
    fn into_update_map(self) -> UpdateMap {
        let mut update_map = UpdateMap::new();
        if let Some(email) = self.email {
            update_map.insert(
                "email".to_string(),
                Some(Value::String(Some(Box::new(email)))),
            );
        }
        if let Some(first_name) = self.first_name {
            update_map.insert(
                "first_name".to_string(),
                Some(Value::String(Some(Box::new(first_name)))),
            );
        }
        if let Some(last_name) = self.last_name {
            update_map.insert(
                "last_name".to_string(),
                Some(Value::String(Some(Box::new(last_name)))),
            );
        }
        if let Some(display_name) = self.display_name {
            update_map.insert(
                "display_name".to_string(),
                Some(Value::String(Some(Box::new(display_name)))),
            );
        }
        if let Some(github_profile_url) = self.github_profile_url {
            update_map.insert(
                "github_profile_url".to_string(),
                Some(Value::String(Some(Box::new(github_profile_url)))),
            );
        }
        if let Some(timezone) = self.timezone {
            update_map.insert(
                "timezone".to_string(),
                Some(Value::String(Some(Box::new(timezone)))),
            );
        }
        update_map
    }
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct UpdatePasswordParams {
    pub new_password: String,
    pub confirm_password: String,
    pub current_password: String,
}

impl IntoUpdateMap for UpdatePasswordParams {
    fn into_update_map(self) -> UpdateMap {
        let mut update_map = UpdateMap::new();
        update_map.insert(
            "password".to_string(),
            Some(Value::String(Some(Box::new(self.new_password)))),
        );
        update_map.insert(
            "confirm_password".to_string(),
            Some(Value::String(Some(Box::new(self.confirm_password)))),
        );
        update_map.insert(
            "current_password".to_string(),
            Some(Value::String(Some(Box::new(self.current_password)))),
        );
        update_map
    }
}
