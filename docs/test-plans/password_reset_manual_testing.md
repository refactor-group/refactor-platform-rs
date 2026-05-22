# Test Plan: Manually Testing Password Reset in Production

Validate the three password-reset endpoints and their rate limiters with
`curl` against `https://myrefactor.com`. Design/threat model:
[`password_reset.md`](../architecture/password_reset.md),
[`throttling.md`](../architecture/throttling.md).

> [!WARNING]
> Real side effects in prod: `/request` sends mail + writes a
> `password_reset_attempts` row; `/complete` changes a real password. Use a
> dedicated test account. The per-email limit can lock it out for 60s or 24h
> (see [§7](#7-resetting-rate-limit-state)).

## 1. Rate-limit layers

| Layer | Scope | Limit | 429 body |
|---|---|---|---|
| nginx `limit_req` | per-IP | ~30/min, burst 20 | nginx HTML, `Server: nginx` |
| `PerIpThrottle` (governor) | per-IP | ~10/min, burst 10 | plain text `Too Many Requests! Wait for Ns` |
| per-email DB limit | email-hash | 1/60s **and** 5/24h | JSON `password_reset_rate_limited` |

Layers nest: from one IP the governor (10/min) trips before nginx (30/min).
Only the per-email limiter returns JSON — use that to tell them apart.

## 2. Setup

```bash
BASE="https://myrefactor.com/api"      # nginx rewrites /api/* → backend /*
TEST_EMAIL="reset-test@yourdomain.com" # mailbox you can read
```

All three endpoints are `POST` + `Content-Type: application/json`.

## 3. Endpoint contract

### `POST /api/password-reset/request` — `{"email":"..."}`

Always `200` (`{"status_code":200,"data":null}`), exists or not.

```bash
curl -sS -i -X POST "$BASE/password-reset/request" \
  -H 'Content-Type: application/json' -d "{\"email\":\"$TEST_EMAIL\"}"
```

| Condition | Status |
|---|---|
| Valid email | `200` |
| Empty / no `@` / >254 chars | `400` |
| Per-email limit tripped | `429` (JSON) |
| DB failure | `503` |

### `POST /api/password-reset/validate` — `{"token":"<43-char>"}`

Token in body, not query string. Does not consume the token.

```bash
TOKEN="paste-43-char-token-from-email"
curl -sS -i -X POST "$BASE/password-reset/validate" \
  -H 'Content-Type: application/json' -d "{\"token\":\"$TOKEN\"}"
```

| Condition | Status | Body |
|---|---|---|
| Valid token | `200` | `{"status_code":200,"data":{"first_name":"...","last_name":"..."}}` |
| Not found / expired / wrong purpose | `400` | `{"error":"invalid_or_expired_token",...}` (all collapsed) |
| Token ≠ 43 chars | `400` | generic `BAD REQUEST` |

### `POST /api/password-reset/complete` — `{"token","password","confirm_password"}`

```bash
curl -sS -i -X POST "$BASE/password-reset/complete" \
  -H 'Content-Type: application/json' \
  -d "{\"token\":\"$TOKEN\",\"password\":\"correct horse battery staple\",\"confirm_password\":\"correct horse battery staple\"}"
```

| Condition | Status |
|---|---|
| Valid token + matching, policy-passing passwords | `200` (returns user; token single-use; no auto-login) |
| `password` != `confirm_password` | `422` |
| Password empty / <12 / >128 chars | `422` |
| Token ≠ 43 chars | `400` |
| Token invalid/expired/wrong purpose | `400` |

## 4. Boundary checks (no account/token needed)

```bash
# 400 — email missing '@'
curl -sS -o /dev/null -w '%{http_code}\n' -X POST "$BASE/password-reset/request" \
  -H 'Content-Type: application/json' -d '{"email":"not-an-email"}'
# 400 — token wrong length
curl -sS -o /dev/null -w '%{http_code}\n' -X POST "$BASE/password-reset/validate" \
  -H 'Content-Type: application/json' -d '{"token":"x"}'
# 422 — confirmation mismatch (valid token length, mismatch caught in domain)
curl -sS -o /dev/null -w '%{http_code}\n' -X POST "$BASE/password-reset/complete" \
  -H 'Content-Type: application/json' \
  -d "{\"token\":\"$(printf 'a%.0s' {1..43})\",\"password\":\"longenoughpassword\",\"confirm_password\":\"different-long-enough\"}"
```

## 5. Per-email limit (easiest to isolate)

Two requests for the same email within 60s trips it:

```bash
curl -sS -o /dev/null -w '1st: %{http_code}\n' -X POST "$BASE/password-reset/request" \
  -H 'Content-Type: application/json' -d "{\"email\":\"$TEST_EMAIL\"}"
curl -sS -i -X POST "$BASE/password-reset/request" \
  -H 'Content-Type: application/json' -d "{\"email\":\"$TEST_EMAIL\"}"
```

Expect `200` then `429` with `password_reset_rate_limited` JSON.

## 6. Per-IP limits (governor → nginx)

Burst from one IP with distinct emails (so per-email doesn't short-circuit):

```bash
for i in $(seq 1 25); do
  curl -sS -o /dev/null -w "req $i: %{http_code}\n" -X POST "$BASE/password-reset/request" \
    -H 'Content-Type: application/json' -d "{\"email\":\"burst+$i@yourdomain.com\"}"
done
```

Expect ~1–10 `200`, ~11–20 governor `429` (plain text), ~21–25 nginx `429`
(HTML). Boundaries shift with replenishment timing. The governor (stricter)
masks nginx per-IP, so confirm nginx via its log line (§8) or in staging.

## 7. Resetting rate-limit state

- **Per-email (durable):** delete the rows. Key = `sha256(lowercase(trim(email)))`.
  ```bash
  printf '%s' "$TEST_EMAIL" | tr 'A-Z' 'a-z' | shasum -a 256   # Linux: sha256sum
  ```
  ```sql
  DELETE FROM refactor_platform.password_reset_attempts WHERE email_hash = '<sha256-hex>';
  ```
- **Governor (in-memory):** backend restart, else ~6s/token.
- **nginx:** `docker compose exec nginx nginx -s reload`, else ~2s/token.

## 8. Logs

```bash
docker compose logs backend | grep '\[password-reset\]'   # raw email never logged
docker compose logs nginx   | grep -i 'limiting requests' # nginx 429s
```

Key backend lines: `rate-limited (min-interval|daily-cap)`,
`reset requested for unknown email ... email_hash_prefix=`,
`reset link issued for user <id>`, `completed password reset`.

## 9. Required prod config

[`service/src/config.rs`](../../service/src/config.rs): `PASSWORD_RESET_EMAIL_TEMPLATE_ID`,
`PASSWORD_RESET_EMAIL_URL_PATH`, `PASSWORD_RESET_TOKEN_EXPIRY_SECONDS`. If the
template ID is missing, `/request` still returns `200` but the background send
fails (`failed to send email to user <id>` instead of `reset link issued`).
