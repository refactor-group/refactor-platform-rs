# MCP Server Setup Guide

Connect your AI client to the Refactor Platform MCP server.

## Prerequisites

1. The backend is running (locally or deployed) — see the [main README](../../README.md)
2. You have a Personal Access Token (PAT) — create one via the REST API:

```bash
# Log in to get a session cookie
curl -X POST http://localhost:4000/login \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "email=your@email.com&password=yourpassword"

# Create a PAT (use the session cookie from login)
curl -X POST http://localhost:4000/users/<your-user-id>/tokens \
  -H "Cookie: <session-cookie-from-login>"
```

The response contains a `token` field — **save it now**, it's shown only once.

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

Use the CLI to add the server:

```bash
claude mcp add --transport http refactor-platform http://localhost:4000/mcp \
  --header "Authorization: Bearer <your-PAT>"
```

Or add to `.mcp.json` in your project root (or `~/.claude/settings.json` for global):

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

Claude Desktop does not natively support `url`-based MCP servers with custom headers. Use the `mcp-remote` bridge:

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
