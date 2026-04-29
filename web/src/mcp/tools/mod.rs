use crate::AppState;
use domain::{
    action, agreement, coaching_relationship, coaching_session, goal, note, user, users, Id,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use serde::Deserialize;
use std::collections::HashMap;

// ── Parameter structs ────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetCoacheeParams {
    /// UUID of the coachee to look up. Required for coaches.
    coachee_id: Option<String>,
    /// Comma-separated list of related data to include: goals, actions, notes
    include: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListSessionsParams {
    /// UUID of the coachee. Required for coaches.
    coachee_id: Option<String>,
    /// Start date filter (YYYY-MM-DD)
    date_from: Option<String>,
    /// End date filter (YYYY-MM-DD)
    date_to: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListActionsParams {
    /// UUID of the coachee. Required for coaches.
    coachee_id: Option<String>,
    /// Filter to actions from a specific session
    coaching_session_id: Option<String>,
    /// Filter by status (not_started, in_progress, completed, on_hold, wont_do)
    status: Option<String>,
    /// Search keyword in action body
    keyword: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetSessionParams {
    /// UUID of the coachee. Required for coaches.
    coachee_id: Option<String>,
    /// UUID of the session. Defaults to most recent.
    session_id: Option<String>,
}

// ── Handler ──────────────────────────────────────────────────────────

/// MCP tool handler. Holds application state for DB access.
/// The authenticated user is NOT stored here — it flows through
/// `RequestContext.extensions` on each tool call via HTTP request Parts.
#[derive(Clone)]
pub(crate) struct McpToolHandler {
    pub(crate) app_state: AppState,
}

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

/// Resolve and authorize a target user ID from an optional coachee_id param.
async fn resolve_target_user(
    db: &sea_orm::DatabaseConnection,
    caller: &users::Model,
    coachee_id: &Option<String>,
) -> Result<Id, McpError> {
    match coachee_id {
        Some(id_str) => {
            let id = Id::parse_str(id_str)
                .map_err(|_| McpError::invalid_params("Invalid coachee_id UUID", None))?;
            if id != caller.id {
                let is_coach = coaching_relationship::is_coach_of(db, caller.id, id)
                    .await
                    .map_err(|e| {
                        McpError::internal_error(format!("Authz check failed: {e}"), None)
                    })?;
                if !is_coach {
                    return Err(McpError::invalid_params(
                        "Forbidden: not a coach of this coachee",
                        None,
                    ));
                }
            }
            Ok(id)
        }
        None => Ok(caller.id),
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, McpError> {
    serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(format!("Serialization error: {e}"), None))
}

// ── Tools ────────────────────────────────────────────────────────────

#[tool_router]
impl McpToolHandler {
    pub(crate) fn new(app_state: AppState) -> Self {
        Self { app_state }
    }

    #[tool(description = "List all coachees for the authenticated coach. No parameters needed.")]
    async fn list_coachees(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user = extract_user(&ctx)?;
        let db = self.app_state.db_conn_ref();

        let relationships = coaching_relationship::find_by_user(db, user.id)
            .await
            .map_err(|e| McpError::internal_error(format!("Query failed: {e}"), None))?;

        let coachee_ids: Vec<Id> = relationships
            .iter()
            .filter(|r| r.coach_id == user.id)
            .map(|r| r.coachee_id)
            .collect();

        if coachee_ids.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No coachees found.",
            )]));
        }

        let coachees = user::find_by_ids(db, &coachee_ids)
            .await
            .map_err(|e| McpError::internal_error(format!("Query failed: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(to_json(
            &coachees,
        )?)]))
    }

    #[tool(
        description = "Get a coachee profile. Coaches must provide coachee_id. \
            Coachees can omit it. Use include (comma-separated: goals,actions,notes) \
            to inline related data."
    )]
    async fn get_coachee(
        &self,
        Parameters(params): Parameters<GetCoacheeParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user = extract_user(&ctx)?;
        let db = self.app_state.db_conn_ref();
        let target_id = resolve_target_user(db, &user, &params.coachee_id).await?;

        let coachee = user::find_by_id(db, target_id)
            .await
            .map_err(|e| McpError::internal_error(format!("Query failed: {e}"), None))?;

        let mut response = serde_json::json!({ "profile": coachee });

        let includes: Vec<&str> = params
            .include
            .as_deref()
            .map(|s| s.split(',').map(|p| p.trim()).collect())
            .unwrap_or_default();

        if includes.contains(&"goals") {
            let rels = coaching_relationship::find_by_user(db, target_id)
                .await
                .unwrap_or_default();
            let mut all_goals = Vec::new();
            for rel in &rels {
                let sids = goal::find_session_ids_by_coaching_relationship_id(db, rel.id)
                    .await
                    .unwrap_or_default();
                if !sids.is_empty() {
                    let grouped = goal::find_goals_grouped_by_session_ids(db, &sids)
                        .await
                        .unwrap_or_default();
                    for gs in grouped.into_values() {
                        all_goals.extend(gs);
                    }
                }
            }
            response["goals"] = serde_json::to_value(&all_goals).unwrap_or_default();
        }

        if includes.contains(&"actions") {
            let actions = action::find_by_user(db, target_id, action::FindByUserParams::default())
                .await
                .unwrap_or_default();
            response["actions"] = serde_json::to_value(&actions).unwrap_or_default();
        }

        if includes.contains(&"notes") {
            let sessions = coaching_session::find_by_user(db, target_id)
                .await
                .unwrap_or_default();
            let mut all_notes = Vec::new();
            for s in &sessions {
                let mut p = HashMap::new();
                p.insert("coaching_session_id".to_string(), s.id.to_string());
                all_notes.extend(note::find_by(db, p).await.unwrap_or_default());
            }
            response["notes"] = serde_json::to_value(&all_notes).unwrap_or_default();
        }

        Ok(CallToolResult::success(vec![Content::text(to_json(
            &response,
        )?)]))
    }

    #[tool(
        description = "List coaching sessions. Coaches must provide coachee_id. \
            Optional date_from/date_to (YYYY-MM-DD)."
    )]
    async fn list_sessions(
        &self,
        Parameters(params): Parameters<ListSessionsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user = extract_user(&ctx)?;
        let db = self.app_state.db_conn_ref();
        let target_id = resolve_target_user(db, &user, &params.coachee_id).await?;

        let mut sessions = coaching_session::find_by_user(db, target_id)
            .await
            .map_err(|e| McpError::internal_error(format!("Query failed: {e}"), None))?;

        if let Some(from) = &params.date_from {
            if let Ok(d) = chrono::NaiveDate::parse_from_str(from, "%Y-%m-%d") {
                sessions.retain(|s| s.date.date() >= d);
            }
        }
        if let Some(to) = &params.date_to {
            if let Ok(d) = chrono::NaiveDate::parse_from_str(to, "%Y-%m-%d") {
                sessions.retain(|s| s.date.date() <= d);
            }
        }

        sessions.sort_by(|a, b| b.date.cmp(&a.date));

        Ok(CallToolResult::success(vec![Content::text(to_json(
            &sessions,
        )?)]))
    }

    #[tool(description = "List actions. Coaches must provide coachee_id. \
            Optional: coaching_session_id, status, keyword.")]
    async fn list_actions(
        &self,
        Parameters(params): Parameters<ListActionsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user = extract_user(&ctx)?;
        let db = self.app_state.db_conn_ref();
        let target_id = resolve_target_user(db, &user, &params.coachee_id).await?;

        let action_params = action::FindByUserParams {
            coaching_session_id: params
                .coaching_session_id
                .as_deref()
                .and_then(|s| Id::parse_str(s).ok()),
            status: params.status.as_deref().map(domain::status::Status::from),
            ..Default::default()
        };

        let mut results = action::find_by_user(db, target_id, action_params)
            .await
            .map_err(|e| McpError::internal_error(format!("Query failed: {e}"), None))?;

        if let Some(kw) = &params.keyword {
            let kw_lower = kw.to_lowercase();
            results.retain(|a| {
                a.action
                    .body
                    .as_deref()
                    .map(|b| b.to_lowercase().contains(&kw_lower))
                    .unwrap_or(false)
            });
        }

        Ok(CallToolResult::success(vec![Content::text(to_json(
            &results,
        )?)]))
    }

    #[tool(
        description = "Get a session with notes, actions, agreements, and goals. \
            Coaches must provide coachee_id. Defaults to most recent session."
    )]
    async fn get_session(
        &self,
        Parameters(params): Parameters<GetSessionParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user = extract_user(&ctx)?;
        let db = self.app_state.db_conn_ref();
        let target_id = resolve_target_user(db, &user, &params.coachee_id).await?;

        let mut sessions = coaching_session::find_by_user(db, target_id)
            .await
            .map_err(|e| McpError::internal_error(format!("Query failed: {e}"), None))?;

        sessions.sort_by(|a, b| b.date.cmp(&a.date));

        let session = if let Some(sid_str) = &params.session_id {
            let sid = Id::parse_str(sid_str)
                .map_err(|_| McpError::invalid_params("Invalid session_id", None))?;
            sessions
                .into_iter()
                .find(|s| s.id == sid)
                .ok_or_else(|| McpError::invalid_params("Session not found", None))?
        } else {
            sessions
                .into_iter()
                .next()
                .ok_or_else(|| McpError::invalid_params("No sessions found", None))?
        };

        let mut np = HashMap::new();
        np.insert("coaching_session_id".to_string(), session.id.to_string());
        let session_notes = note::find_by(db, np).await.unwrap_or_default();

        let session_actions = action::find_by_user(
            db,
            target_id,
            action::FindByUserParams {
                coaching_session_id: Some(session.id),
                ..Default::default()
            },
        )
        .await
        .unwrap_or_default();

        let session_agreements = agreement::find_by_coaching_session_id(db, session.id)
            .await
            .unwrap_or_default();

        let session_goals = goal::find_goals_by_coaching_session_id(db, session.id)
            .await
            .unwrap_or_default();

        let response = serde_json::json!({
            "session": session,
            "notes": session_notes,
            "actions": session_actions,
            "agreements": session_agreements,
            "goals": session_goals,
        });

        Ok(CallToolResult::success(vec![Content::text(to_json(
            &response,
        )?)]))
    }
}

#[tool_handler(
    name = "refactor-platform",
    version = "1.0.0",
    instructions = "MCP server for the Refactor Coaching & Mentoring Platform. \
        Provides tools for coaches and coachees to query coaching data."
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
