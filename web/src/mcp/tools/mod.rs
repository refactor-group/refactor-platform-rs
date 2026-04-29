use crate::AppState;
use rmcp::{tool_handler, tool_router, ServerHandler};

/// MCP tool handler. Holds application state for DB access.
/// The authenticated user is NOT stored here — it flows through
/// `RequestContext.extensions` on each tool call via HTTP request Parts.
#[derive(Clone)]
pub(crate) struct McpToolHandler {
    pub(crate) app_state: AppState,
}

#[tool_router]
impl McpToolHandler {
    pub(crate) fn new(app_state: AppState) -> Self {
        Self { app_state }
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
        // Compile-time check: if McpToolHandler doesn't implement
        // ServerHandler, this function won't compile.
        fn assert_server_handler<T: ServerHandler>() {}
        assert_server_handler::<McpToolHandler>();
    }
}
