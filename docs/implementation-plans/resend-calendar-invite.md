# Resend calendar invite (backend)

Lets a coach resend the calendar invite for a coaching session series or a one-off session, so a coachee who missed or declined part of a series can get the whole series back on their calendar. The frontend counterpart is `refactor-platform-fe/docs/implementation-plans/resend-calendar-invite.md`.

Branch: `resend-calendar-invite`. Built with the overseer/handoff workflow, one phase per commit.

## Decisions

- Recipients are coach and coachee, the same as the original invite. The resend uses the existing "scheduled" templates (`RecurringSessionsScheduled`, `SessionScheduled`), so no new templates or config are needed.
- Every resend bumps `ical_sequence` before it sends. The UID stays the same, so calendars update the existing event rather than adding a copy.
- A series resend carries a **snapshot**: the master VEVENT (RRULE plus an EXDATE for every slot with no member), plus one override VEVENT per member that was moved or resized.
- There is no server-side throttle. The endpoints are coach-only and the frontend puts them behind a confirm dialog.
- Legacy series (members with a NULL `ical_recurrence_id`) get a 409 from the series endpoint. The session endpoint accepts their members, which already go out standalone through `build_session_ics`.
- The send is synchronous. The endpoint answers 502 when the coachee wasn't reached.

## Contract

| Route | Auth | Responses |
|---|---|---|
| `POST /coaching_session_series/:id/resend_invite` | `CoachingSessionSeriesCoachAccess` | 204; 409 when the series is legacy or has no upcoming session; 502 when the coachee wasn't reached |
| `POST /coaching_sessions/:id/resend_invite` | coach of the session's relationship | 204; 409 when the session is a member of a non-legacy series; 502 when the coachee wasn't reached |
| `GET /users/:user_id/coaching_sessions?include=series` | existing | each series member carries `series: { id, last_session_date }`; one-offs have no `series` key |

Both POSTs take no body. A 409 body carries a machine-readable reason (`legacy_series` / `no_upcoming_sessions` / `series_member`) so the frontend can choose the right toast.

## Snapshot rules (B1)

- Slots = `expand_recurrence(rule.start_at, &rule.recurrence)`, each truncated with `SessionDate`, exactly as `bulk_create_recurring` stores them in `ical_recurrence_id`.
- Members = sessions whose `coaching_session_series_id` is this series and whose `ical_recurrence_id` is one of the slots. A member whose recurrence id is not a slot (for example a past session left over from before a series reschedule) is ignored.
- Master: DTSTART = `rule.start_at`, duration = `rule.duration_minutes`, RRULE from `rule.recurrence`, SEQUENCE = `series.ical_sequence`, and EXDATE for every slot that has no member.
- Overrides: one per member where `date != ical_recurrence_id` or `duration_minutes != rule.duration_minutes`. RECURRENCE-ID is the slot, DTSTART is `date`, and SEQUENCE is `max(session.ical_sequence, series.ical_sequence)`. The resend first raises every member to the bumped series value in the same transaction.
- Everything goes in one VCALENDAR with `METHOD:REQUEST` and a single VTIMEZONE block.

## Phases

- [x] B1 (`f46499a3`): `ical.rs` gains EXDATE and multi-VEVENT support; a pure `series_snapshot` builder
- [x] B2 (`6ba8f08f`): `send_series_invite_email` returns `Delivered`
- [x] B3 (`ec624919`): both `resend_invite` endpoints
- [x] B4 (`f6017cdb`): `include=series` with `last_session_date`
- [x] B5 (`ff05caa4`): a series resend raises every member's `ical_sequence` to at least the series' value, and overrides never trail the master. Google Calendar drops an override whose SEQUENCE is lower than its master's; this was found in the live run.

## Follow-ups

- Series dates are spread out in UTC, and the `.ics` is anchored to the coach's profile zone. For a coach in a DST zone, the calendar keeps the local wall time across DST while the app keeps the UTC time, so the two drift by an hour. This predates this feature.
- After two or more series reschedules, recreated sessions start at `ical_sequence` 0 while the series is at 2 or more. A later single-occurrence reschedule can then send an override below its master, which Google drops. A resend now repairs this, but reschedule should seed new sessions at the series value. This predates this feature.
- The "recurring sessions scheduled" email tells the coach that the coachee scheduled the series (`other_user_role`). This predates this feature; the resend inherits it.

- A resend that fails before any send (missing API key or template id) returns 500, not 502. Only a send the provider refuses maps to 502. The frontend shows the same toast for both.

- EXDATE and RECURRENCE-ID values are rendered in the coach's zone from UTC slots. If a series spans a DST change, a client that expands the RRULE in local time can place a slot one hour away from our stored UTC slot. This was already true for the existing RECURRENCE-ID path; it isn't new here.
