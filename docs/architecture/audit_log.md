# Audit log

A durable, append-only record of who changed what in an organization, kept in the
`refactor_platform.audit_events` table. Written by
[`entity_api::audit_event`](../../entity_api/src/audit_event.rs); domain code reaches it through
the `Recorder` trait in [`domain::audit_event`](../../domain/src/audit_event.rs).

## What is recorded

One row per change: the organization, the acting user (null when the system acted), an action,
the changed record (`subject_type`, `subject_id`), a small `details` object and the time.
Events carry ids only, never names or emails. `details` holds small facts such as a kind before
and after; keep it to strings, integers, booleans and ids.

| Action | Subject |
|---|---|
| `relationship_created` | `coaching_relationship` |
| `relationship_deleted` | `coaching_relationship` |
| `relationship_kind_changed` | `coaching_relationship` |
| `member_added` | `user` |
| `member_removed` | `user` |
| `member_role_changed` | `user` |

## Guarantees

- **Same transaction.** A domain write calls `Recorder::record` with its own transaction before
  committing. An event exists exactly when its change committed, and a failed record rolls the
  change back.
- **Outlives its subjects.** The table has no foreign keys, so deleting an organization, a user
  or a relationship leaves its events in place.
- **Append-only.** A trigger refuses every `UPDATE` and `DELETE` on the table, and `TRUNCATE`.
  The app role owns the table, so revoked grants would not hold; the trigger does.
- **Hash-chained per organization.** Each event stores the previous event's hash (`prev_hash`,
  null on an organization's first) and its own `hash`, the lowercase hex SHA-256 of a compact
  JSON array of `prev_hash`, `id`, `organization_id`, `actor_user_id`, `action`,
  `subject_type`, `subject_id`, `details` (every object's keys sorted) and `occurred_at`
  (RFC 3339 UTC, six fractional digits, `Z`). `canonical_encoding` in `entity_api::audit_event`
  is the one definition.
- **One chain order across instances.** Each append takes a per-organization advisory lock
  (`pg_advisory_xact_lock(2, key)`) held until its transaction ends, and stamps its event
  strictly later than the organization's previous one, so `(occurred_at, id)` order is chain
  order even when instance clocks disagree.

## Verifying a chain

`verify_chain(db, organization_id)` walks the organization's events by `(occurred_at, id)` and
returns `Intact { count }`, or `BrokenAt { event_id }` for the first event whose `prev_hash` is
not the previous event's hash or whose recomputed hash differs from the stored one. An edited
event breaks at itself; a removed event breaks at the one after it.

## What it does not protect against

A database superuser can disable the trigger, rewrite rows *and* recompute every hash after
them; the chain then verifies. The chain detects edits by anyone who does not recompute it, and
removal of an organization's newest events is not detectable from the table alone. Keeping a
copy of each organization's latest hash outside the database would close both gaps.
