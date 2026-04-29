# MCP Tools — MVP
The MVP set of tools are all needed to summarize a session for a coach/ee. A typical flow for a coach would be

Ask AI to "What have I been working on with Jane?" ->
`list_coachees` to search for Jane ->
`get_coachee` to learn about Jane ->
`list_sessions` to learn about recent sessions
`get_session` to generate a summary of the most recent session, pulling in other resources like goals, notes, and/or agreements as necessary.

## Coach tools
- `list_coachees` - list coachees associated with a coach.

## Shared tools for Coach & Coachee
A Coach can provide a coachee id for shared tools to filter on that coachee.

- `get_coachee` — profile + aggregated stats for a coachee. Optional `include` to get current goals, actions, and notes. defaults to self when no id.
  - flexibly replaces a lot of index tools like "list_actions" or "list_goals". Filtering was added to support the use case of getting more data about a coachee.
- `list_sessions` - list sessions. optional date range filter.
- `list_actions` — list actions. Filters: session id, keyword (searches body), date range, status. Coaches optionally provide a coachee id (defaults to self for coachees).
- `get_session` — returns structured session data (session + notes + actions + agreements + linked goals) for the client LLM to summarize. No server-side LLM needed. Requires coachee_id for coach users. Optionally accepts session id, defaults to latest.
