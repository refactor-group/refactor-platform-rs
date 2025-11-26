use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct UserCredentials {
    pub email: String,
    pub password: String,
}

impl UserCredentials {
    pub fn parse(input: &str) -> Result<Self> {
        let parts: Vec<&str> = input.split(':').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid credentials format. Expected email:password");
        }
        Ok(Self {
            email: parts[0].to_string(),
            password: parts[1].to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub session_cookie: String,
    pub credentials: UserCredentials,
}

#[derive(Debug, Serialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct UserData {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    data: UserData,
}

pub async fn login(
    client: &Client,
    base_url: &str,
    credentials: &UserCredentials,
) -> Result<AuthenticatedUser> {
    let url = format!("{}/login", base_url);

    let response = client
        .post(&url)
        .form(&LoginRequest {
            email: credentials.email.clone(),
            password: credentials.password.clone(),
        })
        .send()
        .await
        .context("Failed to send login request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Unable to read response body".to_string());
        anyhow::bail!("Login failed: {} - Response: {}", status, body);
    }

    // Extract session cookie
    let session_cookie = response
        .cookies()
        .find(|cookie| cookie.name() == "id")
        .context("No session cookie in response")?
        .value()
        .to_string();

    let api_response: ApiResponse = response
        .json()
        .await
        .context("Failed to parse login response")?;

    Ok(AuthenticatedUser {
        user_id: api_response.data.id,
        session_cookie,
        credentials: credentials.clone(),
    })
}
