# Test Plan: Manually Testing Coaching Note Images (Backend)

Verify `POST /coaching_sessions/{id}/images` and `GET /coaching_session_images/{image_id}`
enforce participation, sniff real content types, honour the size cap, and serve bytes
through a signed redirect (Spaces) or a stream (local filesystem).

Frontend counterpart: `refactor-platform-fe/docs/test-plans/coaching_note_images_manual_testing.md`.
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

### 1.1 Helpers

```sh
BASE=http://localhost:4000
VER='x-version: 1.0.0-beta1'

# Log in and keep a per-actor cookie jar. $1 = jar name, $2 = email, $3 = password.
login() { curl -s -c "/tmp/$1.jar" -X POST "$BASE/login" \
  -H 'content-type: application/x-www-form-urlencoded' \
  --data-urlencode "email=$2" --data-urlencode "password=$3" -o /dev/null -w '%{http_code}\n'; }

# Upload. $1 = jar, $2 = file path
up() { curl -s -i -b "/tmp/$1.jar" -H "$VER" -F "file=@$2" \
  "$BASE/coaching_sessions/$SESSION/images"; }

# Fetch an image. $1 = jar, $2 = image id. No x-version, no -L: we want to SEE the redirect.
img() { curl -s -i -b "/tmp/$1.jar" "$BASE/coaching_session_images/$2"; }

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

**Pass:** 201. JSON `ApiResponse` whose `data` carries `id`, `coaching_session_id` equal to
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

**Pass:** 201. Both participants may add images; this is a shared note.

### Case 3: the declared content type is ignored

```sh
up coach /tmp/liar.jpg
```

**Pass:** 201 with `mime_type: "image/png"`, not `image/jpeg`. The sniffed type wins and is
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
`coaching_session_note_images` table.

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
less than** `NOTE_IMAGE_PRESIGN_TTL_SECONDS` (default 900) so a cached redirect can never
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

**Pass:** a clean 404 or 500 with a JSON body. Not a panic, not a hung request. The frontend
renders its "image isn't available" state from this.

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

**Pass:** the `coaching_session_note_images` rows for that session are gone (FK cascade).
The **objects remain in storage** — that is the known, accepted v1 gap, not a bug. Confirm by
listing the bucket prefix or the local directory.

```sql
SELECT count(*) FROM refactor_platform.coaching_session_note_images
WHERE coaching_session_id = '<throwaway session id>';  -- expect 0
```

## 3. Cleanup

```sh
rm -f /tmp/small.png /tmp/evil.svg /tmp/liar.jpg /tmp/doc.pdf /tmp/big.png /tmp/got.png
rm -f /tmp/coach.jar /tmp/coachee.jar /tmp/outsider.jar
rm -rf ./.local-object-store        # local backend artifacts
```

Restore any relationship you revoked in Case 12 and delete test objects from the Spaces
bucket if you ran Case 8.
