# MCP Tools — Exhaustive Tool List
Prefer not deleting through mcp for now, destructively risky.

## Coach-Only
1. `list_coachees` — all coachees for this coach
2. `get_coachee` — profile + aggregated stats for one coachee
3. `list_overdue_actions` — overdue actions across all coachees
4. `create_session` — schedule a session
5. ~~`update_session` — change date, meeting URL~~ - rescheduling is better done in the UI where you see a calendar
6. ~~`delete_session` — cancel a session~~ - too destructive for LLM-initiated calls, do this in the UI
7. `create_goal` — create goal for a coachee
8. ~~`update_goal` — edit goal title/body~~ - editing body text via MCP is awkward; status changes are the high-value operation
9. ~~`delete_goal` — remove a goal~~ - too destructive for LLM-initiated calls, do this in the UI
10. `create_action` — create action item with optional assignees and goal link
11. ~~`update_action` — edit action body/due date~~ - editing body text via MCP is awkward; status changes are the high-value operation
12. ~~`delete_action` — remove action~~ - too destructive for LLM-initiated calls, do this in the UI
13. `create_note` — add note to a session
14. ~~`update_note` — edit a note~~ - low value via MCP, notes are typically written once
15. `create_agreement` — add agreement to a session
16. ~~`update_agreement` — edit agreement~~ - low value via MCP, agreements are typically written once
17. ~~`delete_agreement` — remove agreement~~ - too destructive for LLM-initiated calls, do this in the UI
18. `weekly_digest` — summary across all coachees (generative)
19. `prepare_for_session` — pre-session brief (generative)
20. ~~`suggest_goals` — suggest goals based on session history (generative)~~ - requires LLM on the server, post-MVP

## Coachee-Only
21. ~~`get_my_coach` — coach profile for a relationship~~ - the coachee already knows their coach; low utility
22. ~~`create_goal` — coachee creates own goal~~ - coachees can use the UI for this; the value of MCP for coachees is reading, not writing

## Shared (both roles, coach specifies coachee, coachee auto-scoped)
23. `list_sessions` — sessions by date range, sort
24. `get_session` — full session detail with notes, actions, agreements, goals via `include` param
25. `list_goals` — goals filterable by status
26. `list_actions` — actions filterable by session, goal, status, due date
27. ~~`list_notes` — notes for a session~~ - folds into `get_session` via `include` parameter
28. ~~`list_agreements` — agreements for a session~~ - folds into `get_session` via `include` parameter
29. `update_goal_status` — change goal status
30. `update_action_status` — change action status
31. `get_session` — session recap (generative)
