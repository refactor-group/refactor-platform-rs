# rmcp Integration

How the [`rmcp`](https://docs.rs/rmcp/latest/rmcp/) SDK ([`modelcontextprotocol/rust-sdk`](https://github.com/modelcontextprotocol/rust-sdk)) integrates with the Axum backend.

## Dependency

```toml
rmcp = { version = "0.8", features = ["server", "macros", "transport-streamable-http-server"] }
```

`rmcp` re-exports [`schemars`](https://docs.rs/schemars/latest/schemars/) and [`serde`](https://docs.rs/serde/latest/serde/) — no separate `schemars` dependency needed in `web/Cargo.toml`.

## Architecture

### Request flow through layers

```
HTTP Request (Authorization: Bearer <PAT>)
  │
  ▼
Axum Router (.layer → require_pat_auth middleware)
  │  ✅ Has HTTP request, headers, extensions
  │  Validates PAT → resolves users::Model
  │  Inserts users::Model into request extensions
  │  401 if invalid
  │
  ▼
StreamableHttpService (Tower Service)
  │  Receives HTTP request, parses JSON-RPC
  │  Propagates HTTP request Parts into RequestContext.extensions
  │
  ▼
McpToolHandler (ServerHandler trait)
  │  Owns: app_state (DB connection, config)
  │  Dispatches to #[tool] methods via self
  │
  ▼
Tool method (e.g. list_coachees)
   Extracts user from context.extensions.get::<axum::http::request::Parts>()
   Accesses self.app_state for DB
```

### User context propagation

`rmcp` propagates the HTTP request [`Parts`](https://docs.rs/http/latest/http/request/struct.Parts.html) into [`RequestContext.extensions`](https://docs.rs/rmcp/latest/rmcp/service/struct.RequestContext.html). This means data inserted into HTTP request extensions by Axum middleware is accessible inside tool handlers.

The auth middleware inserts `users::Model` into request extensions. Tool handlers extract it:

```rust
// In the tool handler:
if let Some(axum_parts) = context.extensions.get::<axum::http::request::Parts>() {
    if let Some(user) = axum_parts.extensions.get::<users::Model>() {
        // user is the authenticated caller
    }
}
```

This pattern was discovered in [`apollo-mcp-server`](https://github.com/apollographql/apollo-mcp-server/blob/ff28390b/crates/apollo-mcp-server/src/server/states/running.rs), which uses the same approach to propagate validated OAuth tokens from Axum middleware into `rmcp` tool handlers. It is not documented in `rmcp`'s own docs or examples — their [`simple_auth_streamhttp`](https://github.com/modelcontextprotocol/rust-sdk/blob/main/examples/servers/src/simple_auth_streamhttp.rs) example only gates access without passing user identity into tools.

This eliminates the need for `Arc<RwLock<...>>` bridging or storing the user on the handler struct. The `McpToolHandler` only needs `app_state` — the user flows through the request context on every call.

## Stateless mode (MVP)

[`StreamableHttpServerConfig`](https://docs.rs/rmcp/latest/rmcp/transport/streamable_http_server/tower/struct.StreamableHttpServerConfig.html) `{ stateful_mode: false, .. }` — every POST is self-contained. No session ID, no persistent handler. The factory creates a fresh `McpToolHandler` per request.

Stateful mode (`stateful_mode: true`, the default) creates persistent sessions where the handler lives across multiple requests. Deferred to post-MVP for SSE streaming.

## Auth integration

PAT middleware wraps the [`Router`](https://docs.rs/axum/latest/axum/struct.Router.html) containing [`nest_service`](https://docs.rs/axum/latest/axum/struct.Router.html#method.nest_service), not the Tower service directly:

```rust
let mcp_service = StreamableHttpService::new(factory, session_manager, config);

Router::new()
    .nest_service("/mcp", mcp_service)
    .layer(middleware::from_fn_with_state(app_state, require_pat_auth))
```

- [`.layer()`](https://docs.rs/axum/latest/axum/struct.Router.html#method.layer) applies to all routes and nested services in the Router.
- [`route_layer`](https://docs.rs/axum/latest/axum/struct.Router.html#method.route_layer) does NOT work here — it only applies to `.route()` registrations.
- This matches rmcp's official [`simple_auth_streamhttp`](https://github.com/modelcontextprotocol/rust-sdk/blob/main/examples/servers/src/simple_auth_streamhttp.rs) example.

## Tool definition pattern

Tools use three macros together:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListCoacheesParams {
    // empty — caller identified via PAT in request context
}

#[tool_router]
impl McpToolHandler {
    #[tool(description = "List coachees for the authenticated coach")]
    async fn list_coachees(
        &self,
        Parameters(params): Parameters<ListCoacheesParams>,
        context: RequestContext<RoleServer>,
    ) -> String {
        // Extract user from HTTP request parts propagated by rmcp
        let parts = context.extensions.get::<axum::http::request::Parts>().unwrap();
        let user = parts.extensions.get::<users::Model>().unwrap();
        // self.app_state — DB access
    }
}

#[tool_handler(name = "refactor-platform", version = "1.0.0")]
impl ServerHandler for McpToolHandler {}
```

- [`#[tool_router]`](https://docs.rs/rmcp/latest/rmcp/attr.tool_router.html) — generates tool dispatch and `list_tools` from annotated methods.
- [`#[tool]`](https://docs.rs/rmcp/latest/rmcp/attr.tool.html) — marks a method as an MCP tool, generates JSON Schema from the `Parameters<T>` type.
- [`#[tool_handler]`](https://docs.rs/rmcp/latest/rmcp/attr.tool_handler.html) — auto-generates the [`ServerHandler`](https://docs.rs/rmcp/latest/rmcp/handler/server/trait.ServerHandler.html) impl with server info and tool capabilities. Use this when you need custom name/version.
- [`Parameters<T>`](https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/src/handler/server/wrapper.rs) — wrapper extractor. `T` must derive `schemars::JsonSchema` + `Deserialize`.

## Handler struct

```rust
struct McpToolHandler {
    app_state: AppState,     // DB connection, config
}
```

The handler struct does not hold the user. The authenticated user is extracted from [`RequestContext.extensions`](https://docs.rs/rmcp/latest/rmcp/service/struct.RequestContext.html) on each tool call (see [User context propagation](#user-context-propagation) above). The factory closure only needs `app_state`:

```rust
let app_state = app_state.clone();
StreamableHttpService::new(
    move || Ok(McpToolHandler { app_state: app_state.clone() }),
    session_manager,
    config,
)
```

## OAuth (future)

`rmcp` has built-in [OAuth 2.1 support](https://github.com/modelcontextprotocol/rust-sdk/blob/main/docs/OAUTH_SUPPORT.md) via the `auth` feature flag:
- PKCE (S256), dynamic client registration, token refresh, scope upgrade
- Protected Resource Metadata discovery ([RFC 9728](https://datatracker.ietf.org/doc/html/rfc9728))
- [`AuthClient`](https://docs.rs/rmcp/latest/rmcp/transport/auth/struct.AuthClient.html) wraps `reqwest` with automatic token management

This is client-side support. For the server side, we would implement authorization endpoints (authorize, token, metadata) in Axum, following the pattern in [`complex_auth_streamhttp`](https://github.com/modelcontextprotocol/rust-sdk/blob/main/examples/servers/src/complex_auth_streamhttp.rs). The PAT auth middleware is designed to support a second credential type — OAuth would resolve to the same `users::Model`, reusing the existing authorization layer.
