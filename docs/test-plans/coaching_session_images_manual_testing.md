# Test Plan: Manually Testing Coaching Note Images (Backend)

Verify `POST /coaching_sessions/{id}/images` and `GET /coaching_session_images/{image_id}`
enforce participation, sniff real content types, honour the size cap, and serve bytes
through a signed redirect (Spaces) or a stream (local filesystem). Section 2b covers the
deferred-deletion signals, `DELETE /coaching_session_images/{image_id}` and
`POST /coaching_session_images/{image_id}/restore`, and the purge job that eventually acts on them.

Frontend counterpart: `refactor-platform-fe/docs/test-plans/coaching_session_images_manual_testing.md`.
Implementation plan: `docs/implementation-plans/coaching-note-images-backend.md`.

> [!IMPORTANT]
> The mock suite covers byte sniffing, the allowlist, the size cap, the local store's path
> traversal guard and presigned-URL shape. It never exercises a real bucket, a real browser
> `<img>` load, or a real cookie on a subresource request. **Cases 6, 7, 12, 13 and 16 are
> the only proof those work.** Case 13 in particular cannot be replaced by an automated test.

> [!NOTE]
> Requires backend phases B3 (routes) and B4 (env wiring). Until B3 lands, every case here
> returns 404 because no route is registered. Sections 1 and 2 are still correct as written.

## 1. Prerequisites

- Backend on branch `144-coaching-note-images`, running on `:4000`, DB seeded
  (`cargo run --bin seed_db`) and migrations applied.
- A coaching session whose relationship has a coach and a coachee, plus **a third user who
  is a participant in neither** (needed for Case 11).
- Every request needs the `x-version` header **except** the image GET, which deliberately
  omits `CompareApiVersion` because a browser `<img>` tag cannot send custom headers.

### 1.0 Before a deployed run (previews or production)

The code is inert until the GitHub environment is populated. Until then the service boots fine and
the image endpoints return **503 with a warning** — the loud failure, deliberately chosen over
silently accepting uploads that would be lost. Set these first:

- **Secrets:** `SPACES_ACCESS_KEY_ID`, `SPACES_SECRET_ACCESS_KEY`
- **Vars:** `SPACES_ENDPOINT`, `SPACES_REGION`, `SPACES_BUCKET`
  (`COACHING_SESSION_IMAGE_MAX_BYTES` and `COACHING_SESSION_IMAGE_PRESIGN_TTL_SECONDS` are optional; blank uses the
  in-code defaults of 10 MB and 900s.)

`OBJECT_STORE_BACKEND` is **not** one of these — it is hardcoded to `spaces` in
`deploy_to_do.yml` on purpose. An unset value would be stripped by `Config::sanitize_empty_env`
and fall back to the `local` default, writing images into the container filesystem where a
redeploy destroys them. Do not make it configurable.

**Local runs need none of this** — the default `local` backend writes to a gitignored directory.

### 1.1 Helpers

```sh
BASE=http://localhost:4000
VER='x-version: 1.0.0'

# Log in and keep a per-actor cookie jar. $1 = jar name, $2 = email, $3 = password.
login() { curl -s -c "/tmp/$1.jar" -X POST "$BASE/login" \
  -H 'content-type: application/x-www-form-urlencoded' \
  --data-urlencode "email=$2" --data-urlencode "password=$3" -o /dev/null -w '%{http_code}\n'; }

# Upload. $1 = jar, $2 = file path
up() { curl -s -i -b "/tmp/$1.jar" -H "$VER" -F "file=@$2" \
  "$BASE/coaching_sessions/$SESSION/images"; }

# Fetch an image. $1 = jar, $2 = image id. No x-version, no -L: we want to SEE the redirect.
img() { curl -s -i -b "/tmp/$1.jar" "$BASE/coaching_session_images/$2"; }

# Signal a removal. $1 = jar, $2 = image id. These DO send x-version: our own API module calls them.
del() { curl -s -i -X DELETE -b "/tmp/$1.jar" -H "$VER" "$BASE/coaching_session_images/$2"; }

# Signal an undo. $1 = jar, $2 = image id.
undel() { curl -s -i -X POST -b "/tmp/$1.jar" -H "$VER" "$BASE/coaching_session_images/$2/restore"; }

# The removal mark for an image, read straight from the DB. $1 = image id.
mark() { psql "$DATABASE_URL" -tAc \
  "select coalesce(deleted_at::text, 'NULL') from refactor_platform.coaching_session_images where id = '$1';"; }

login coach james.hodapp@gmail.com password
login coachee calebbourg2@gmail.com password
login outsider <third user's email> password
SESSION=<session id>
```

### 1.2 Fixture files

Generate these once. Do not commit them.

```sh
cd /tmp
# A real 2x3 PNG
printf '\x89PNG\r\n\x1a\n' > /dev/null   # sanity only; use a real file below
sips -s format png --resampleWidth 64 /System/Library/CoreServices/DefaultDesktop.heic \
  --out small.png >/dev/null 2>&1 || echo "use any small PNG as /tmp/small.png"

# Scriptable SVG. Must be refused.
cat > evil.svg <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><script>alert(1)</script></svg>
SVG

# A PNG renamed to .jpg, to prove the declared type is ignored
cp small.png liar.jpg

# A PDF: a format `infer` recognizes but that is off the allowlist
printf '%%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\ntrailer<</Root 1 0 R>>\n' > doc.pdf

# 11 MB of zeros, over the 10 MB default cap
mkdir -p big && head -c 11534336 /dev/zero > big.png
```

## 2. Cases

Each case: command, expected status, expected headers or body shape.

### Case 1: coach uploads a PNG

```sh
up coach /tmp/small.png
```

**Pass:** **HTTP 200** with `"status_code": 201` inside the body. That looks wrong and is not —
successful creates in this codebase return the envelope's status in the body, not on the response
line (compare `topic_controller::create`). Only the *error* paths below use real HTTP statuses.
JSON `ApiResponse` whose `data` carries `id`, `coaching_session_id` equal to
`$SESSION`, `mime_type: "image/png"`, a `byte_size` matching the file, and non-null
`width`/`height`. **`storage_key` must NOT appear** — it is `#[serde(skip)]` so the bucket
layout never reaches a client. Save the id:

```sh
IMAGE=<id from the response>
```

### Case 2: the coachee can upload to the same session

```sh
up coachee /tmp/small.png
```

**Pass:** HTTP 200 with `"status_code": 201` in the body, as in Case 1. Both participants may add
images; this is a shared note.

### Case 3: the declared content type is ignored

```sh
up coach /tmp/liar.jpg
```

**Pass:** HTTP 200 with `"status_code": 201` in the body and `mime_type: "image/png"`, not
`image/jpeg`. The sniffed type wins and is
what gets stored on the object.

### Case 4: SVG is refused

```sh
up coach /tmp/evil.svg
```

**Pass:** 415. SVG is scriptable XML; serving it from our own origin would be stored XSS
against a signed-in coach. There is no configuration that should make this succeed.

### Case 5: a recognized but non-allowlisted format is refused

```sh
up coach /tmp/doc.pdf
```

**Pass:** 415. Distinct from Case 4: `infer` *identifies* a PDF, so this is the allowlist
doing the work rather than the sniff coming back empty.

### Case 6: over the size cap

```sh
up coach /tmp/big.png
```

**Pass:** 413. Note the body limit layer may reject before the handler reads the part, so an
empty body with a 413 status is acceptable. Confirm nothing was written to the bucket or the
`coaching_session_images` table.

### Case 7: fetching an image — local backend

With `OBJECT_STORE_BACKEND=local`:

```sh
img coach $IMAGE
```

**Pass:** 200, `content-type: image/png`, the PNG bytes in the body,
`cache-control: private, max-age=600` and `vary: Cookie`. The local backend cannot presign,
so it streams.

### Case 8: fetching an image — Spaces backend

Restart with `OBJECT_STORE_BACKEND=spaces` and the four `SPACES_*` variables set, then
re-run Case 1 to create an object in the bucket and:

```sh
img coach $IMAGE
```

**Pass:** **302**, `location:` a `digitaloceanspaces.com` URL carrying `X-Amz-Signature=`
and `response-cache-control=private%2C%20max-age%3D86400%2C%20immutable`.
`cache-control: private, max-age=600` on the redirect itself, and that max-age **must be
less than** `COACHING_SESSION_IMAGE_PRESIGN_TTL_SECONDS` (default 900) so a cached redirect can never
outlive its own signature. Follow it and confirm the bytes arrive:

```sh
curl -s -o /tmp/got.png -w '%{http_code}\n' "$(img coach $IMAGE | awk '/^location:/{print $2}' | tr -d '\r')"
cmp /tmp/got.png /tmp/small.png && echo "bytes identical"
```

### Case 9: the image GET needs no `x-version`

Already true of the `img` helper, which sends none. To make it explicit, confirm the *upload*
fails without it while the fetch still succeeds:

```sh
curl -s -o /dev/null -w 'upload without version: %{http_code}\n' \
  -b /tmp/coach.jar -F "file=@/tmp/small.png" "$BASE/coaching_sessions/$SESSION/images"
img coach $IMAGE | head -1
```

**Pass:** the upload returns 400 (or the version-mismatch status), the fetch returns 200/302.
**If the fetch starts failing here, someone has added `CompareApiVersion` to the read
handler and every image in the product is broken** — an `<img>` tag cannot send that header.

### Case 10: unauthenticated fetch

```sh
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/coaching_session_images/$IMAGE"
```

**Pass:** 401. No cookie, no bytes.

### Case 11: a non-participant is refused

```sh
up outsider /tmp/small.png | head -1
img outsider $IMAGE | head -1
```

**Pass:** both 403 or 404. A user outside the coaching relationship can neither add an image
to the session nor read one already there, **even holding a valid image id**. This is the
case that proves the id is not a bearer token.

### Case 12: a revoked participant loses access

Remove the coachee from the relationship (or archive it), then:

```sh
img coachee $IMAGE | head -1
```

**Pass:** 403 or 404. Access is evaluated per request, not captured at upload time. Restore
the relationship afterwards.

### Case 13: storage unconfigured

Restart with `OBJECT_STORE_BACKEND=spaces` and **no** `SPACES_*` credentials.

```sh
up coach /tmp/small.png | head -1
```

**Pass:** the service **still boots** (check the log for the "object storage disabled"
warning naming the missing variable) and the upload returns **503**, not a panic and not a
500. Everything unrelated to images keeps working — confirm by listing coaching sessions.

### Case 14: a missing object

Delete the stored object directly (remove the file under `OBJECT_STORE_LOCAL_PATH`, or the
key in the bucket) while leaving the DB row, then:

```sh
img coach $IMAGE | head -1
```

**Pass:** **404**, not 500. A row outliving its bytes is a routine condition — a half-finished
purge, a bucket restored from an older snapshot — and a 500 would page someone for it. Not a
panic, not a hung request. The frontend renders its "image isn't available" state from this.

Check the log while you are here: it should carry the "Object store has no object at ..."
warning and nothing at error level.

### Case 15: path traversal through the storage key

Covered by `local_store_rejects_path_traversal_keys` in
`domain/src/gateway/object_storage.rs`, which the overseer mutation-tested. No manual step —
listed so nobody assumes it is untested. To re-confirm the test has teeth, delete the guard
in `LocalObjectStore::resolve` and check that exactly that test fails.

### Case 16: the row survives a session delete correctly

```sh
# note the image ids for a throwaway session, then delete the session
curl -s -X DELETE -b /tmp/coach.jar -H "$VER" "$BASE/coaching_sessions/<throwaway session id>"
```

**Pass:** the `coaching_session_images` rows for that session are gone (FK cascade).
The **objects remain in storage** — that is the known, accepted v1 gap, not a bug. Confirm by
listing the bucket prefix or the local directory.

```sql
SELECT count(*) FROM refactor_platform.coaching_session_images
WHERE coaching_session_id = '<throwaway session id>';  -- expect 0
```

## 2b. Deferred deletion

Removing an image from a note happens inside an opaque Yjs blob, so the frontend *signals* it and
the backend only marks the row. The bytes survive a grace period
(`COACHING_SESSION_IMAGE_GRACE_PERIOD_HOURS`, default 168) so an undo can resurrect the image with
the same id. These cases prove the mark, the survival, and the eventual destruction.

Implementation plan: `docs/implementation-plans/coaching-session-image-deferred-delete.md`.

Upload a fresh image first and keep its id:

```sh
up coach /tmp/small.png
IMAGE=<id from the response>
```

### Case 17: a removal marks the row

```sh
del coach $IMAGE
mark $IMAGE
```

**Pass:** HTTP 200 with `"status_code": 200` in the body and the image model in `data`
(`deleted_at` is `#[serde(skip)]` and deliberately never crosses the wire, so read it from SQL).
`mark` prints a timestamp, not `NULL`.

### Case 18: a marked image is still served

```sh
img coach $IMAGE | head -1
```

**Pass:** 200 or 302, exactly as before the removal. **This is the case that proves undo will
work.** If the row exists the object exists, so the fetch never consults `deleted_at`. A 404 here
means someone filtered soft-deleted rows out of the read handler and silently broke undo.

### Case 19: a second removal does not extend the grace window

```sh
FIRST=$(mark $IMAGE); echo "first: $FIRST"
del coach $IMAGE | head -1
SECOND=$(mark $IMAGE); echo "second: $SECOND"
[ "$FIRST" = "$SECOND" ] && echo "timestamp preserved"
```

**Pass:** 200, and the two timestamps are **identical**. Both participants observe the same removal
and both may signal it; a repeat call that reset the clock would keep the image alive forever.

### Case 20: an undo clears the mark

```sh
undel coach $IMAGE
mark $IMAGE
img coach $IMAGE | head -1
```

**Pass:** the restore returns 200 with `"status_code": 200`, `mark` prints `NULL`, and the fetch
still returns 200/302. Restoring an image that is already live is a no-op, not an error — run
`undel coach $IMAGE` twice to confirm.

### Case 21: a non-participant can neither remove nor restore

```sh
del outsider $IMAGE | head -1
undel outsider $IMAGE | head -1
```

**Pass:** both 403 or 404, and `mark $IMAGE` is unchanged by either call. The image id lives
forever inside note bytes, so it must not act as a bearer token on these routes either.

### Case 22: a forced purge destroys a marked image

Restart the backend with the grace period collapsed so the job fires immediately:

```sh
COACHING_SESSION_IMAGE_GRACE_PERIOD_HOURS=0 \
COACHING_SESSION_IMAGE_PURGE_POLL_MINUTES=1 \
cargo run
```

Upload two images. Mark **one** of them and leave the other alone:

```sh
up coach /tmp/small.png     # DOOMED=<id>
up coach /tmp/small.png     # LIVE=<id>
DOOMED=<first id>; LIVE=<second id>
del coach $DOOMED | head -1
# wait for one poll tick, then:
mark $DOOMED
img coach $DOOMED | head -1
```

**Pass:** after a tick, `mark` returns **no row at all**, the fetch returns **404**, and the object
is gone from storage (`ls -R ./.local-object-store`, or list the bucket prefix). The object goes
first and the row second, so a storage failure leaves the row for the next tick to retry — an
orphaned object with nothing pointing at it is the failure this ordering avoids.

### Case 23: the purge cannot touch a live image

In the **same** forced-purge run as Case 22, immediately after that tick:

```sh
mark $LIVE                  # expect NULL
img coach $LIVE | head -1   # expect 200 or 302
```

**Pass:** the live image's row is still there with `deleted_at` null, and it is still served. **This
is the safety guard end to end:** with the grace period at zero, every row is old enough to purge,
so the only thing keeping this image alive is the `deleted_at IS NOT NULL` predicate. If this case
fails, the job is destroying images users can still see. Restore the default grace period before
moving on.

## 3. Cleanup

```sh
rm -f /tmp/small.png /tmp/evil.svg /tmp/liar.jpg /tmp/doc.pdf /tmp/big.png /tmp/got.png
rm -f /tmp/coach.jar /tmp/coachee.jar /tmp/outsider.jar
```

> [!WARNING]
> **Do not delete `./.local-object-store` while image rows still exist.** The rows point at those
> files; removing the directory leaves every one of them broken, and the notes referencing them
> render a permanent "image isn't available" state. Check first:
>
> ```sh
> psql "$DATABASE_URL" -tAc "select count(*) from refactor_platform.coaching_session_images;"
> ```
>
> Only when that count is `0` is the directory safe to remove:
>
> ```sh
> rm -rf ./.local-object-store        # local backend artifacts
> ```
>
> If the count is non-zero and you want a clean slate, delete the rows first (or reseed the
> database), then remove the directory.

Restore any relationship you revoked in Case 12, restore the default
`COACHING_SESSION_IMAGE_GRACE_PERIOD_HOURS` if you ran Case 22, and delete test objects from the
Spaces bucket if you ran Case 8.
