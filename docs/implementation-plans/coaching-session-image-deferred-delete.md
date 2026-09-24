# Deferred Image Deletion — Design and Test Strategy

Extends `coaching-note-images-backend.md`. Frontend counterpart:
`refactor-platform-fe/docs/plans/images-in-coaching-notes-fe.md`.

## The problem

Removing an image from a coaching note is an edit inside an opaque Yjs blob, so the backend never
learned about it. Rows and objects accumulated forever. v1 accepted that deliberately, because the
obvious fix — deleting on removal — would break undo: Cmd-Z restores the node with the same
`imageId`, and the bytes would already be gone.

## The design

**The frontend can detect every removal.** A ProseMirror plugin diffs `transaction.before` against
`transaction.doc` on each transaction and reports `coachingNoteImage` nodes that disappeared. That
covers the delete control, backspace, cut and select-all-delete uniformly — not just the NodeView's
own handler.

**Destruction is deferred, so undo stays free.** A removal marks `deleted_at`; the row and object
survive. Undo re-inserts the node with the same `imageId`, which still resolves, so **the undo path
needs no re-upload, no new id, and no attribute mutation**. A background job destroys the object and
the row only once the grace period has elapsed.

```
remove node ──▶ DELETE  ──▶ deleted_at = now()      (bytes untouched, GET still serves)
undo        ──▶ restore ──▶ deleted_at = NULL
                                │
                     grace period elapses
                                ▼
                    purge job: delete object, then row
```

### Decisions

| | |
|---|---|
| **`GET` is unchanged** | If the row exists the object exists, so the fetch does not consult `deleted_at` at all. Undo works during the grace window for free; after a purge the row is gone and it 404s. |
| **Both endpoints idempotent** | Two participants observe the same removal. `DELETE` on an already-deleted row keeps the *original* timestamp, so repeat calls cannot extend the grace window. |
| **Local-origin transactions only** | A remote delete replicates to the other client, whose editor also sees the node vanish. Filtering to local origin keeps it to one call; idempotency covers the race regardless. |
| **The purge refuses `deleted_at IS NULL`** | The guard is in the query *and* pinned by a test. A bug in the signal path then leaks storage instead of destroying live images. |
| **Object first, then row** | A failed object delete leaves the row, so the next tick retries. Deleting the row first would orphan the object with nothing pointing at it. |
| **Signal is best-effort** | A failed `DELETE`, a closed tab, a client that never reconnects — each leaks one row. Bounded and invisible. That is the accepted residual. |

### `deleted_at` never crosses the wire

The column is `#[serde(skip)]` on the entity, so it appears in no request or response. That is
deliberate: it is purge bookkeeping, not part of the client contract, and the frontend branches on
the `200` alone.

One consequence for tests, found during the B7 review: a controller test **cannot** assert the
transition at the wire boundary, because the only field that changes is invisible. Asserting "it
returned 200" would pass even if the handler echoed the extractor's row and never called the domain
at all. The controller tests therefore give the domain-returned row a distinct `updated_at`, seed
the pre-update row in the *opposite* state, and assert the whole `data` payload — which fails the
moment a handler stops delegating. The `deleted_at` transition itself is covered by the entity_api
unit tests and end to end by the manual plan.

### Config

| Variable | Default |
|---|---|
| `COACHING_SESSION_IMAGE_GRACE_PERIOD_HOURS` | `168` (7 days) |
| `COACHING_SESSION_IMAGE_PURGE_POLL_MINUTES` | `60` |

The job follows the existing `domain::jobs::Job` trait and registers on the same `Scheduler` as
`session_reminder` and `password_reset`. It builds its own `ObjectStore` via
`object_storage::from_config`, mirroring `session_reminder::Sweep::from_config`, and is **not
scheduled at all** when storage is unconfigured.

## Test strategy

The property that matters is *an image a user can still get back is never destroyed*. Tests are
chosen so that each one fails if a specific part of that guarantee is removed.

### Backend — unit / `MockDatabase`

| Test | Fails when |
|---|---|
| `DELETE` sets `deleted_at` on a null row | the endpoint is a no-op |
| `DELETE` on an already-deleted row **preserves the original timestamp** | repeat calls extend the grace window |
| restore clears `deleted_at` | undo cannot resurrect |
| restore on a live row is a no-op | |
| purge selects only rows with `deleted_at IS NOT NULL` **and** older than the grace period | **the safety guard is removed** — the headline test |
| purge deletes the object **before** the row | a storage failure orphans the object |
| purge reports `Outcome::partial` when an object delete fails | a failed tick looks idle |
| the job is not constructed when storage is unconfigured | it ticks pointlessly and logs errors |

### Backend — controller / authorization

| Test | Fails when |
|---|---|
| a non-participant cannot `DELETE` (403/404) | the extractor is dropped |
| a non-participant cannot restore | |
| unauthenticated `DELETE` is 401 | |
| **`GET` still serves an image whose `deleted_at` is set** | someone "helpfully" filters soft-deleted rows out of the fetch and silently breaks undo |

### Frontend — real-TipTap integration

| Test | Fails when |
|---|---|
| removing an image node via `deleteSelection` calls `DELETE` once with its `imageId` | the diff misses the delete control path |
| removing it via **backspace** calls `DELETE` | the plugin only watches the NodeView, which was the original mistake |
| **undo after a delete calls restore with the same `imageId`** | undo does not resurrect server-side |
| a transaction that removes **no** image calls nothing | the plugin fires on unrelated edits |
| removing **two** images in one transaction calls `DELETE` for both | the diff returns only the first |
| a **remote** (y-sync origin) removal calls nothing | both clients fire duplicates |
| the document is unchanged when `DELETE` fails | a failed call corrupts the note |

### End-to-end

Playwright cannot reach this editor (live collab JWT + websocket), so the end-to-end proof is the
manual plans in both repos, extended for this feature:

- **Backend plan:** soft-delete via `curl`, confirm `GET` still serves, confirm the row survives,
  confirm a restore clears the flag. Then a forced purge with the grace period set to zero, proving
  the object and row go, and that a row with `deleted_at IS NULL` is untouched by the same tick.
- **Frontend plan:** delete an image, confirm the row gains `deleted_at` in SQL, press Cmd-Z,
  confirm the image renders again *and* the flag clears. Repeat via backspace. Confirm a second
  browser's copy does not fire a duplicate delete.

The highest-value manual case is the one no automated test covers: **delete an image, reload the
page, and confirm it is still gone from the note but the row is still recoverable in SQL** — the
whole point of deferring destruction.
