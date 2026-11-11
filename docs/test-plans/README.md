# Backend Test Plans

Procedures for checking a feature against a running backend, by hand or driven by an agent.
Each plan lists its full prerequisites; this index only says what it covers and what would stop
you or cost you.

Pick a plan by what your change touches, check its **Needs** column before starting, and
report results in the PR, commit or coordination board, never in the plan.

## Index

| Plan | Covers | Run when | Needs | Frontend counterpart |
|---|---|---|---|---|
| [coaching_session_images_manual_testing.md](coaching_session_images_manual_testing.md) | Note image upload, fetch, access, deferred delete and purge | Image handlers, object storage, the purge job | Local object store; Case 8 needs a real Spaces bucket. Destructive | [coaching_session_images_manual_testing.md](https://github.com/refactor-group/refactor-platform-fe/blob/main/docs/test-plans/coaching_session_images_manual_testing.md) |
| [docs-collab-server-local-e2e.md](docs-collab-server-local-e2e.md) | Self-hosted collab server on one machine: provisioning, auth, sync, persistence | docs-collab-server, collab tokens, document create/delete | Collab server and FE running locally with matching secrets; two browsers | |
| [docs-collab-server-preview-e2e.md](docs-collab-server-preview-e2e.md) | The same on a deployed PR preview, plus restarts and graceful flush | docs-collab-server deployment, verifying a preview | A healthy `pr-<N>` preview and SSH to its host. Destructive host steps need the operator | |
| [ics-calendar-invites-manual-test-plan.md](ics-calendar-invites-manual-test-plan.md) | `.ics` invites on session and series create, reschedule and cancel | The invite builder, session/series email paths, timezones | **Sends real email** unless `RESEND_BASE_URL` points at a mock; real calendar clients | [resend_calendar_invite_manual_testing.md](https://github.com/refactor-group/refactor-platform-fe/blob/main/docs/test-plans/resend_calendar_invite_manual_testing.md) |
| [local_recall_webhook_tunnel_manual_testing.md](local_recall_webhook_tunnel_manual_testing.md) | The Recall webhook tunnel script and a real recording through it | The tunnel script, Recall webhooks, local Recall setup | Own Recall workspace and ngrok domain (`docs/setup.md` §4); a second Google account | |
| [multi_org_membership_manual_testing.md](multi_org_membership_manual_testing.md) | Adding an existing user to another org, removal, lookup, re-adding | Org membership, user lookup, the added-to-org email | Two orgs, four users. Scenarios E and F **send real email**. Destructive | |
| [oauth_disconnect_revokes_token_manual_testing.md](oauth_disconnect_revokes_token_manual_testing.md) | OAuth disconnect revoking the Google or Zoom grant | OAuth disconnect, token revocation | A real Google account; revokes its grant. Zoom is blocked (no config anywhere) | |
| [organization_member_role_change_manual_testing.md](organization_member_role_change_manual_testing.md) | Changing an org member's role: authz, last-admin guard, audit | The role endpoints, org-admin authz, role auditing | Seeded DB; writes permanent `user_role_changes` rows | [member_role_change_manual_testing.md](https://github.com/refactor-group/refactor-platform-fe/blob/main/docs/test-plans/member_role_change_manual_testing.md) |
| [password_reset_manual_testing.md](password_reset_manual_testing.md) | Password reset endpoints and their rate limits | Reset endpoints, throttling, nginx limits | **Production** only. **Sends real email**; can lock the test account | |
| [relationship_access_manual_testing.md](relationship_access_manual_testing.md) | Who may do what through a coaching relationship (rs#399 baseline): REST access matrix by entity, operation and caller (coach, coachee, same-org non-participant, other org, org admin, SuperAdmin, removed member) for relationships, sessions, series, notes, agreements, actions, goals, topics, transcripts, recordings, images, meeting links, search, collab token, users, orgs and SSE; `[tightened]` rules from the relationship authorization fixes; `[membership]` rules for removed members (own lists, emails, 409 `coachee_not_a_member` / `assignee_not_a_member`); background job and email recipients; wire inventory of coach/coachee terms | Any change to relationship, session, series, action, agreement, note, goal or topic authorization, `*_access` extractors or `protect/` middleware, own-list queries, email recipients, or org membership rules; each rs#399 phase | Local backend and migrations; the plan's fixture (two orgs, its cast of users including a removed member, R1-R3, S1-S3, SR1, SR3) built with its curl helpers; cookie jars; psql. Email rows need a mock Resend (`RESEND_BASE_URL`) or inboxes you own, else the dev `.env` SENDS REAL EMAIL. Removes and re-adds org roles. By hand (curl); phase 0b turns its coach/coachee rows into automated tests | refactor-platform-fe: docs/test-plans/relationship_access_manual_testing.md |
| [remove_org_member_revokes_access_manual_testing.md](remove_org_member_revokes_access_manual_testing.md) | A removed coach or coachee loses access; nothing is destroyed. Scenario I predates re-adding by org admins | Member removal, access extractors, SSE recipients | A relationship with real history; two browsers. Destructive | |
| [search_capability_manual_testing.md](search_capability_manual_testing.md) | `GET /search`: query syntax, filters, pagination, access rules | Search, a new searchable type, access rules search mirrors | The plan's SQL fixture; Scenario M can't be undone (rerun the fixture) | |
| [session_reminder_emails_manual_testing.md](session_reminder_emails_manual_testing.md) | The session reminder sweep: exactly-once claims, re-arming, suppression | The reminder job, its config or migration | Mock Resend; without it the dev `.env` **sends real email**. Some cases need two replicas | |
| [transcript_download_manual_testing.md](transcript_download_manual_testing.md) | Transcript download: `Accept` negotiation, speaker filter, errors. Its speaker-role cases predate attribution by meeting account | Transcript download, the speaker filter | A completed transcription (SQL fixture or a real recording) | |
| [transcript_speaker_attribution_manual_testing.md](transcript_speaker_attribution_manual_testing.md) | Who said what: coach by meeting account, coachee by elimination, guests | Attribution, participant storage, Recall participant handling | Part A: real recordings, in production. Part B: local SQL fixture | |
| [transcript_suggestions_manual_testing.md](transcript_suggestions_manual_testing.md) | Suggested actions end to end: extraction, review, permissions, retries | Suggestion extraction, review endpoints, the analysis sweep | Working Recall setup and real recordings; `ASSEMBLY_AI_API_KEY`. **Paid** model calls | [transcript_suggestions_ui_manual_testing.md](https://github.com/refactor-group/refactor-platform-fe/blob/main/docs/test-plans/transcript_suggestions_ui_manual_testing.md) |

## Automated test guides

Not manual plans: how to run and extend automated suites.

| Guide | Covers | Run when | Needs |
|---|---|---|---|
| [sqlite_integration_testing.md](sqlite_integration_testing.md) | The in-memory SQLite test tier vs the mock suite | Tests that depend on which rows a query selects; adding an entity | Nothing; plain `cargo test` |
| [transcript_suggestions_eval.md](transcript_suggestions_eval.md) | The suggested-action eval against `baseline.json` | Any prompt, model, verification or due-date change | `ASSEMBLY_AI_API_KEY`. **Paid**: about $2 a run |

## Adding a plan

- Name it `<feature>_manual_testing.md`, and add a row here in the same PR.
- State the prerequisites up front: email on or off, fixture, logins and keys (by env var name,
  never the value).
- For a feature with a frontend plan, name it as the counterpart and don't re-run its checks.
- Keep the plan static: it is a procedure, not a log.
