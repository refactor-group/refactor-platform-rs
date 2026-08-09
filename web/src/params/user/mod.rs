pub(crate) mod action;
pub(crate) mod coaching_relationship;
pub(crate) mod coaching_session;
pub(crate) mod goal;

// Re-export user profile update params for backward compatibility
use domain::{users, users::Role, Id, IntoUpdateMap, UpdateMap};
use sea_orm::Value;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for an exact-match user lookup by email.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct LookupParams {
    pub email: String,
}

/// Body for granting a user a role within an organization.
///
/// Unknown fields are rejected so a misspelled key fails loudly rather than
/// silently granting the wrong role.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachRoleParams {
    pub role: Role,
    /// Coach to assign in the same transaction as the membership.
    pub coach_id: Option<Id>,
}

/// Body for creating a new member of an organization.
///
/// The user fields are flattened so a client that sends only them still
/// deserializes. serde rejects `deny_unknown_fields` alongside `flatten`, so
/// unknown keys are ignored rather than refused here.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMemberParams {
    #[serde(flatten)]
    pub user: users::Model,
    /// Coach to assign in the same transaction as the new account.
    pub coach_id: Option<Id>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct UpdateParams {
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub display_name: Option<String>,
    pub github_profile_url: Option<String>,
    pub timezone: Option<String>,
    /// Per-coach default coaching-session duration in minutes (1..=480).
    /// Validated in entity_api via the `Duration` newtype. `i16` matches
    /// both the storage type and the newtype's inner representation.
    pub default_coaching_session_duration_minutes: Option<i16>,
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
        if let Some(default_duration) = self.default_coaching_session_duration_minutes {
            update_map.insert(
                "default_coaching_session_duration_minutes".to_string(),
                Some(Value::SmallInt(Some(default_duration))),
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

#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct CompleteSetupParams {
    pub token: String,
    pub password: String,
    pub confirm_password: String,
}

impl IntoUpdateMap for CompleteSetupParams {
    fn into_update_map(self) -> UpdateMap {
        let mut update_map = UpdateMap::new();
        update_map.insert(
            "token".to_string(),
            Some(Value::String(Some(Box::new(self.token)))),
        );
        update_map.insert(
            "password".to_string(),
            Some(Value::String(Some(Box::new(self.password)))),
        );
        update_map.insert(
            "confirm_password".to_string(),
            Some(Value::String(Some(Box::new(self.confirm_password)))),
        );
        update_map
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct PasswordResetRequestParams {
    pub email: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct PasswordResetCompleteParams {
    pub token: String,
    pub password: String,
    pub confirm_password: String,
}

impl IntoUpdateMap for PasswordResetCompleteParams {
    fn into_update_map(self) -> UpdateMap {
        let mut update_map = UpdateMap::new();
        update_map.insert(
            "token".to_string(),
            Some(Value::String(Some(Box::new(self.token)))),
        );
        update_map.insert(
            "password".to_string(),
            Some(Value::String(Some(Box::new(self.password)))),
        );
        update_map.insert(
            "confirm_password".to_string(),
            Some(Value::String(Some(Box::new(self.confirm_password)))),
        );
        update_map
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
