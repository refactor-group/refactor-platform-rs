use crate::AppState;
use domain::{coaching_relationship, user, users, Id};
use rmcp::{
    model::{CallToolResult, Content},
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};

/// MCP tool handler. Holds application state for DB access.
/// The authenticated user is NOT stored here — it flows through
/// `RequestContext.extensions` on each tool call via HTTP request Parts.
#[derive(Clone)]
pub(crate) struct McpToolHandler {
    pub(crate) app_state: AppState,
}

/// Extract the authenticated user from the MCP request context.
///
/// The PAT auth middleware inserts `users::Model` into HTTP request extensions.
/// `rmcp` propagates HTTP `Parts` into `RequestContext.extensions`.
fn extract_user(ctx: &RequestContext<RoleServer>) -> Result<users::Model, McpError> {
    let parts = ctx
        .extensions
        .get::<axum::http::request::Parts>()
        .ok_or_else(|| McpError::internal_error("Missing HTTP request parts in context", None))?;

    parts
        .extensions
        .get::<users::Model>()
        .cloned()
        .ok_or_else(|| McpError::internal_error("Missing authenticated user in context", None))
}

#[tool_router]
impl McpToolHandler {
    pub(crate) fn new(app_state: AppState) -> Self {
        Self { app_state }
    }

    /// List all coachees for the authenticated coach.
    /// Returns coachee profiles (id, name, email) for each coaching relationship
    /// where the authenticated user is the coach.
    #[tool(description = "List all coachees for the authenticated coach. \
        No parameters needed — the coach is identified via the PAT token. \
        Returns an array of coachee profiles.")]
    async fn list_coachees(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user = extract_user(&ctx)?;
        let db = self.app_state.db_conn_ref();

        let relationships = coaching_relationship::find_by_user(db, user.id)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("Failed to query relationships: {e}"), None)
            })?;

        // Filter to relationships where the authenticated user is the coach
        let coachee_ids: Vec<Id> = relationships
            .iter()
            .filter(|r| r.coach_id == user.id)
            .map(|r| r.coachee_id)
            .collect();

        if coachee_ids.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No coachees found. You may not have any coaching relationships as a coach.",
            )]));
        }

        let coachees = user::find_by_ids(db, &coachee_ids).await.map_err(|e| {
            McpError::internal_error(format!("Failed to query coachees: {e}"), None)
        })?;

        let json = serde_json::to_string_pretty(&coachees)
            .map_err(|e| McpError::internal_error(format!("Serialization error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler(
    name = "refactor-platform",
    version = "1.0.0",
    instructions = "MCP server for the Refactor Coaching & Mentoring Platform. \
        Provides tools for coaches and coachees to query coaching data including \
        coachees, sessions, goals, actions, and notes."
)]
impl ServerHandler for McpToolHandler {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_implements_server_handler() {
        fn assert_server_handler<T: ServerHandler>() {}
        assert_server_handler::<McpToolHandler>();
    }
}
