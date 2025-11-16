use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

pub struct ApiClient {
    client: Client,
    base_url: String,
}

#[derive(Debug, Clone)]
pub struct TestEnvironment {
    pub relationship_id: String,
    pub session_id: String,
}

impl ApiClient {
    pub fn new(client: Client, base_url: String) -> Self {
        Self { client, base_url }
    }

    pub async fn setup_test_environment(
        &self,
        coach_session: &str,
        _coachee_session: &str,
        coach_id: &str,
        coachee_id: &str,
    ) -> Result<TestEnvironment> {
        // Create coaching relationship
        let relationship = self
            .create_coaching_relationship(coach_session, coach_id, coachee_id)
            .await?;

        let relationship_id = relationship["id"]
            .as_str()
            .context("No relationship ID in response")?
            .to_string();

        // Create coaching session
        let session = self
            .create_coaching_session(coach_session, &relationship_id)
            .await?;

        let session_id = session["id"]
            .as_str()
            .context("No session ID in response")?
            .to_string();

        Ok(TestEnvironment {
            relationship_id,
            session_id,
        })
    }

    async fn create_coaching_relationship(
        &self,
        session_cookie: &str,
        coach_id: &str,
        coachee_id: &str,
    ) -> Result<Value> {
        let url = format!("{}/coaching_relationships", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Cookie", format!("session_id={}", session_cookie))
            .json(&json!({
                "coach_id": coach_id,
                "coachee_id": coachee_id,
            }))
            .send()
            .await
            .context("Failed to create coaching relationship")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to create relationship: {}", response.status());
        }

        response.json().await.context("Failed to parse response")
    }

    async fn create_coaching_session(
        &self,
        session_cookie: &str,
        relationship_id: &str,
    ) -> Result<Value> {
        let url = format!("{}/coaching_sessions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Cookie", format!("session_id={}", session_cookie))
            .json(&json!({
                "coaching_relationship_id": relationship_id,
                "date": "2024-01-01",
            }))
            .send()
            .await
            .context("Failed to create coaching session")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to create session: {}", response.status());
        }

        response.json().await.context("Failed to parse response")
    }

    pub async fn create_action(
        &self,
        session_cookie: &str,
        coaching_session_id: &str,
        title: &str,
    ) -> Result<Value> {
        let url = format!("{}/actions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Cookie", format!("session_id={}", session_cookie))
            .json(&json!({
                "coaching_session_id": coaching_session_id,
                "title": title,
                "description": "Created by SSE test tool",
                "status": "not_started",
            }))
            .send()
            .await
            .context("Failed to create action")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to create action: {}", response.status());
        }

        response.json().await.context("Failed to parse response")
    }

    pub async fn update_action(
        &self,
        session_cookie: &str,
        action_id: &str,
        title: &str,
    ) -> Result<Value> {
        let url = format!("{}/actions/{}", self.base_url, action_id);

        let response = self
            .client
            .put(&url)
            .header("Cookie", format!("session_id={}", session_cookie))
            .json(&json!({
                "title": title,
            }))
            .send()
            .await
            .context("Failed to update action")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to update action: {}", response.status());
        }

        response.json().await.context("Failed to parse response")
    }

    pub async fn delete_action(&self, session_cookie: &str, action_id: &str) -> Result<()> {
        let url = format!("{}/actions/{}", self.base_url, action_id);

        let response = self
            .client
            .delete(&url)
            .header("Cookie", format!("session_id={}", session_cookie))
            .send()
            .await
            .context("Failed to delete action")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to delete action: {}", response.status());
        }

        Ok(())
    }

    pub async fn force_logout(&self, admin_session_cookie: &str, user_id: &str) -> Result<()> {
        let url = format!("{}/admin/force_logout/{}", self.base_url, user_id);

        let response = self
            .client
            .post(&url)
            .header("Cookie", format!("session_id={}", admin_session_cookie))
            .send()
            .await
            .context("Failed to force logout")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to force logout: {}", response.status());
        }

        Ok(())
    }
}
