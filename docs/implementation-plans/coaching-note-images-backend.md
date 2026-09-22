# Coaching Note Images — Backend Plan

Frontend issue: [refactor-platform-fe#144](https://github.com/refactor-group/refactor-platform-fe/issues/144).
Frontend plan: `refactor-platform-fe/docs/plans/images-in-coaching-notes-fe.md`.
Branch: `144-coaching-note-images`.

## Context

Coaches need to paste screenshots into Coaching Notes to stop maintaining a parallel Google Doc.
The platform has **no object storage and no file upload of any kind** today, so this is the first
binary asset pipeline. It is built generically — a trait, a config block, a key convention — so org
logos and profile pictures reuse it later rather than growing a second one.

## The constraint that shapes everything

Note content is a Yjs CRDT persisted as one opaque `BYTEA` blob per session in
`refactor_platform.collab_documents`. Consequences:

1. **No base64 in the document.** It would be re-broadcast on every join and re-persisted on every
   debounce, and the editor is gated on that sync (`SYNC_TIMEOUT_MS = 10_000` on the frontend).
2. **Anything written into an image node is permanent and un-greppable** — it cannot contain an
   expiring signature or a hostname we might change. The frontend therefore stores an **image id**
   and resolves the URL at render time, which is why the read endpoint is image-scoped.
3. **The backend never learns when an image is removed from a note**, because that edit happens
   inside opaque CRDT bytes. See "Accepted gaps".

## Decisions

| | |
|---|---|
| **Storage** | Local filesystem in local dev; **DO Spaces** in PR previews and production, selected by `OBJECT_STORE_BACKEND`. |
| **Abstraction** | A small in-house `ObjectStore` trait in `domain/src/gateway/object_storage.rs`, **not** the `object_store` crate. It brings its own HTTP stack, which conflicts with the repo's hard `.use_rustls_tls()` rule (features are unioned across the workspace). `rusty-s3` is sans-IO — it signs, we send with our own rustls client — so the TLS rule stays enforceable. |
| **Serving** | `GET /coaching_session_images/:image_id` → **302** to a presigned Spaces GET when the backend can sign; **stream the bytes** when it can't (local backend). |
| **Upload** | Browser → backend `multipart`, so the server can sniff magic bytes and enforce the cap for real. |
| **Limits** | 10 MB. png/jpeg/webp/gif by magic bytes. SVG rejected. |

## Why the read endpoint omits `CompareApiVersion`

An `<img>` tag cannot send the `X-Version` header. Every other endpoint takes that extractor; this
one deliberately does not, and the handler carries a comment saying so.

## Why cookie auth works on an `<img>` (an invariant)

The session cookie is `SameSite=Lax` and host-only (`web/src/lib.rs:142-144`). An `<img>` is a
subresource, so a genuinely cross-site load would arrive unauthenticated. It doesn't: production
serves both apps from one origin (`nginx/conf.d/refactor-platform.conf` routes `/api/` → backend,
`/` → Next.js on `myrefactor.com`); PR previews use the same path routing; locally `:3000` and
`:4000` are the same *site* (SameSite ignores ports).

**Moving the API to a different registrable domain breaks every note image with no client-side fix.**

## Rejection is a value, not an error variant

`DomainErrorKind::Validation(String)` maps to 422 in `web/src/error.rs`, and there is no 413/415
mapping. The frontend distinguishes "too large" from "unsupported type", and the standards forbid
adding error variants. So `domain::coaching_session_note_image::inspect_image` returns
`Result<InspectedImage, ImageRejection>` where `ImageRejection` is a plain domain **value type** —
the same shape as `entity::duration::OutOfRange`, which the standards name as the sanctioned
pattern. The controller matches it to a status code.

## Endpoints

```
POST /coaching_sessions/{coaching_session_id}/images
  CompareApiVersion, CoachingSessionAccess, AuthenticatedUser, Multipart
  multipart/form-data, part "file"
  201 { status_code, data: { id, coaching_session_id, mime_type, byte_size, width, height, created_at } }
  400 no file part · 403/404 no access · 413 oversize · 415 unsupported/SVG · 503 storage unconfigured

GET /coaching_session_images/{image_id}
  CoachingSessionNoteImageAccess only — deliberately NO CompareApiVersion
  302 → presigned GET, or 200 with the bytes when the backend cannot sign
  Cache-Control: private, max-age=600   (MUST be < the presign TTL)
  Vary: Cookie
```

## Phases

| Phase | Scope | Status |
|---|---|---|
| **B1** | Config fields + `ObjectStore` trait, local + Spaces impls, wired onto `AppState` | **done** (`0b15fb8b`) |
| **B2** | Migration, entity, entity_api, domain (validation + create/find) | pending |
| **B3** | Extractor, controller, router registration, `DefaultBodyLimit`, utoipa | pending |
| **B4** | `.env*`, docker-compose ×3, `docs/setup.md`, preview nginx body-size check | pending |

## Local backend notes (from B1)

- `LocalObjectStore` writes a `<key>.content-type` sidecar beside each object so reads return the
  stored type faithfully instead of re-sniffing.
- Its key guard is two-layered: a lexical check rejecting absolute keys and non-`Normal` components,
  then a canonicalization check that the deepest existing ancestor still resolves inside the root —
  which catches a symlink escaping the tree, something a lexical check cannot see.
- Local filesystem I/O failures use the existing `Internal(InternalErrorKind::Other)`; neither
  `Config` nor `External(Network)` describes a failed disk write honestly. No new error variants
  were added.
- `Bucket::new` needs a `url::Url`; we use `reqwest::Url`, a re-export of the same crate, rather
  than adding a `url` dependency.

## Storage key convention

`coaching-sessions/{session_id}/notes/{image_id}.{ext}` — session-prefixed so a future
session-delete can drop a whole prefix, and so logos can sit under a sibling prefix in the same
bucket.

## Accepted gaps

- **Orphaned objects.** Removing an image from a note is invisible to the backend. v1 deletes
  nothing. Any future reaper **must** keep undo working (deleting a node must never delete bytes) —
  give it a grace period. The only correct reference check is loading the Y.Doc and walking it for
  `imageId` attrs, which is Node-side work.
- **Pre-existing, separate issue:** `collab_documents` has no foreign key to `coaching_sessions`,
  so deleting a session already orphans its note blob.
