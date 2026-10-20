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
- Overrides: one per member where `date != ical_recurrence_id` or `duration_minutes != rule.duration_minutes`. RECURRENCE-ID is the slot, DTSTART is `date`, SEQUENCE is the session's `ical_sequence`.
- Everything goes in one VCALENDAR with `METHOD:REQUEST` and a single VTIMEZONE block.

## Phases

- [ ] B1: `ical.rs` gains EXDATE and multi-VEVENT support; a pure `series_snapshot` builder
- [ ] B2: `send_series_invite_email` returns `Delivered`
- [ ] B3: both `resend_invite` endpoints
- [ ] B4: `include=series` with `last_session_date`

## Follow-ups

- EXDATE and RECURRENCE-ID values are rendered in the coach's zone from UTC slots. If a series spans a DST change, a client that expands the RRULE in local time can place a slot one hour away from our stored UTC slot. This was already true for the existing RECURRENCE-ID path; it isn't new here.
