# MCP Server Setup Guide

Connect your AI client to the Refactor Platform MCP server.

## Prerequisites

1. The backend is running (locally or deployed) — see the [main README](../../README.md)
2. You have a Personal Access Token (PAT) — create one via the REST API:

```bash
# Log in: save the session cookie and capture your user ID from the response
curl -s -c cookies.txt -X POST http://localhost:4000/login \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -H "x-version: 1.0.0-beta1" \
  -d "email=your@email.com&password=yourpassword" | jq -r '.data.id'
# → prints your user ID (e.g. 9df9e69d-a123-4a3a-aa2c-557995cf3df3)

# Create a PAT, reusing the cookie jar from login
curl -s -b cookies.txt -X POST http://localhost:4000/users/<your-user-id>/tokens \
  -H "x-version: 1.0.0-beta1"
```

The response contains a `token` field — **save it now**, it's shown only once.

> **API version header:** every endpoint requires an `x-version` header. The only valid value today is `1.0.0-beta1` — this is the *API contract version*, pinned in [`service/src/config.rs`](../../service/src/config.rs), and is intentionally decoupled from the crate's build version (which moves forward independently). Omitting the header returns HTTP 400 `` `x-version` header is missing ``; sending the wrong value returns `` `x-version` header is not a valid API version ``.

## Connection Details

| Setting | Value |
|---------|-------|
| URL | `http://localhost:4000/mcp` (local) or `https://myrefactor.com/mcp` (production) |
| Transport | Streamable HTTP |
| Auth | `Authorization: Bearer <your-PAT>` |

## Warp

In Warp, go to **Settings → MCP Servers → Add Server**, then choose **Streamable HTTP** and enter:

- **Name**: `refactor-platform`
- **URL**: `http://localhost:4000/mcp`
- **Headers**: `Authorization: Bearer <your-PAT>`

Or paste this JSON block in the MCP server config:

```json
{
  "refactor-platform": {
    "url": "http://localhost:4000/mcp",
    "headers": {
      "Authorization": "Bearer <your-PAT>"
    }
  }
}
```

> **Note:** If you have secret redaction enabled (Settings → Privacy → Secret redaction), Warp will block saving the config because the PAT is detected as a secret. Temporarily disable secret redaction, save the MCP server config, then re-enable it.

## Claude Code

Use the CLI to add the server (recommended):

```bash
claude mcp add --transport http refactor-platform http://localhost:4000/mcp \
  --header "Authorization: Bearer <your-PAT>"
```

This writes to `~/.claude.json` scoped to the current project directory, keeping the PAT out of the repo. **Prefer this over a checked-in `.mcp.json`** — a PAT committed to version control is a credential leak.

If you need a portable config (e.g., `~/.claude/settings.json` for global use, or a local-only `.mcp.json` you've added to `.gitignore`):

```json
{
  "mcpServers": {
    "refactor-platform": {
      "type": "http",
      "url": "http://localhost:4000/mcp",
      "headers": {
        "Authorization": "Bearer <your-PAT>"
      }
    }
  }
}
```

## Claude Desktop

Claude Desktop's `claude_desktop_config.json` only accepts stdio servers (`command` + `args`) — it does not natively support `url`-based MCP servers with custom headers. Use the `mcp-remote` bridge:

Edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):

```json
{
  "mcpServers": {
    "refactor-platform": {
      "command": "npx",
      "args": [
        "-y",
        "mcp-remote",
        "http://localhost:4000/mcp",
        "--header",
        "Authorization: Bearer <your-PAT>"
      ]
    }
  }
}
```

Restart Claude Desktop after editing.

### Windows: argv space-escaping workaround

On Windows, Claude Desktop mangles spaces inside argv values, which silently corrupts the `Authorization: Bearer <token>` string and produces a 401 with no obvious cause. Move the bearer prefix into an env var and drop the space after `:` in the arg:

```json
{
  "mcpServers": {
    "refactor-platform": {
      "command": "npx",
      "args": [
        "-y",
        "mcp-remote",
        "http://localhost:4000/mcp",
        "--header",
        "Authorization:${AUTH_HEADER}"
      ],
      "env": {
        "AUTH_HEADER": "Bearer <your-PAT>"
      }
    }
  }
}
```

Spaces survive intact inside `env` values; the `:`-only header arg avoids the buggy argv path entirely. (See the [`mcp-remote` README](https://github.com/geelen/mcp-remote#readme) for the upstream note on this issue.)

### Why not the "Custom Connectors" UI?

Recent Claude Desktop versions expose **Settings → Connectors → Add custom connector** for native remote MCP servers. That path is OAuth-oriented — the UI walks the user through an OAuth handshake against the server — and is the right choice for production SaaS connectors. For a self-hosted dev backend authenticated by a static PAT, the `mcp-remote` bridge above is simpler and more reliable.

## Cursor

Edit `.cursor/mcp.json` in your project root or `~/.cursor/mcp.json` globally:

```json
{
  "mcpServers": {
    "refactor-platform": {
      "url": "http://localhost:4000/mcp",
      "headers": {
        "Authorization": "Bearer <your-PAT>"
      }
    }
  }
}
```

## Verify

After connecting, ask your AI client:

> List my coachees

It should call `list_coachees` and return data. If you get a 401, check that your PAT is valid and the `Authorization` header is formatted correctly (`Bearer ` + token, with a space).

## Available Tools

| Tool | Description |
|------|-------------|
| `list_coachees` | List all coachees for the authenticated coach |
| `get_coachee` | Coachee profile with optional goals, actions, notes |
| `list_sessions` | Coaching sessions with optional date range filter |
| `list_actions` | Actions with session, status, and keyword filters |
| `get_session` | Full session bundle (notes, actions, agreements, goals) |

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| 401 Unauthorized | PAT is missing, expired, or deactivated. Generate a new one. |
| Connection refused | Backend isn't running. Start it with `cargo run`. |
| No tools listed | The `initialize` handshake succeeded but `tools/list` returned empty. Check the server logs for errors. |
| Forbidden on tool call | You're querying a coachee you don't have a coaching relationship with. |
