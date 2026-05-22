# Password Reset Architecture

User-initiated password reset via a single-use, email-delivered magic link. Reuses the magic-link token infrastructure that powers the welcome/setup flow ([authentication_error_flow.md](authentication_error_flow.md), [email_notifications.md](email_notifications.md)), with a new `purpose` column on `magic_link_tokens` to prevent cross-flow token reuse.

## Endpoints

All three are unauthenticated — by design, a user who has forgotten their password cannot authenticate.

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/password-reset/request` | Generate token, email reset link. Always 200 (enumeration-safe). |
| `POST` | `/password-reset/validate` (token in JSON body) | Non-destructive validity check for FE state machine. Returns sanitized user (`first_name`, `last_name` only). |
| `POST` | `/password-reset/complete` | Consume token, set new password. Returns full user. |

Wire format is the source-of-truth `PasswordResetEndpoints` v1 contract on the cross-repo coordinator blackboard.

## End-to-End Flow

```mermaid
sequenceDiagram
    actor User
    participant FE as Frontend
    participant BE as Backend (handler)
    participant BG as Backend (spawned task)
    participant DB as PostgreSQL
    participant MS as MailerSend
    participant Inbox as User's Inbox

    User->>FE: Click "Forgot password?"
    FE->>BE: POST /password-reset/request {email}
    Note over BE: Sync critical path<br/>(both branches identical)
    BE->>DB: rate-limit query (by email_hash)
    BE->>DB: record attempt (by email_hash)
    BE->>BG: tokio::spawn(process_in_background)
    BE->>BE: pad to HANDLER_TARGET_DURATION_MS (150ms)
    BE-->>FE: 200 {data: null}
    FE-->>User: "If an account exists, check your inbox"

    Note over BG: Path-distinguishing work<br/>(response already sent)
    BG->>DB: find user by email
    alt user exists
        BG->>DB: delete prior PasswordReset tokens for user
        BG->>DB: insert new token (purpose=PasswordReset, sha256_hash, exp=now+30m)
        BG->>MS: send_email (template, {first_name, password_reset_url})
        MS-->>Inbox: email with reset link
    else user not found
        BG->>BG: log WARN; discard
    end

    User->>Inbox: Click link
    Inbox->>FE: GET /reset-password/<token>
    FE->>BE: POST /password-reset/validate {token}
    BE->>DB: lookup token_hash, check exp, check purpose=PasswordReset
    BE-->>FE: 200 {first_name, last_name} OR 400 invalid_or_expired_token
    FE-->>User: render form OR error state

    User->>FE: Submit new password
    FE->>BE: POST /password-reset/complete {token, password, confirm_password}
    BE->>DB: BEGIN
    BE->>DB: validate_token (re-check exp + purpose)
    BE->>DB: delete all PasswordReset tokens for user
    BE->>DB: update users.password = argon2(new_password)
    BE->>DB: COMMIT
    BE-->>FE: 200 {user}
    FE-->>User: redirect to login
```

## Token Lifecycle

Two distinct tables are involved in the password-reset flow, with very different semantics:

| Table | Role | Key | Cardinality | Lifecycle |
|---|---|---|---|---|
| `magic_link_tokens` (purpose = `PasswordReset`) | **State table** — the currently-redeemable token | `user_id` (FK to users) | At most one row per user (delete-then-create on issuance) | Created on `/request` for known users only, deleted on `/complete` or on next issuance |
| `password_reset_attempts` | **Append-only audit log** — every request attempt | `email_hash` (SHA-256 of normalized email; no FK) | One row per request, recorded for **both** known and unknown emails | Created on `/request`, only removed by the ops sweep job |

Two structural decisions are encoded above: (1) the **state-vs-audit split** prevents the daily-cap from being unreachable (see [Rate-Limit Audit Separate from Token State](#rate-limit-audit-separate-from-token-state)); (2) the **email-hash key on the audit table** (not user_id) makes the rate limit apply uniformly to unknown-email and known-user paths (see [Enumeration Safety on Both Paths](#enumeration-safety-on-both-paths)).

| Stage | Action | Effect on `magic_link_tokens` | Effect on `password_reset_attempts` |
|---|---|---|---|
| Step 1 — `/request` rate-limit check (every request, before user lookup) | Hash email; query attempts by `email_hash` over the 60s and 24h windows | No effect | No effect (read-only) |
| Step 2 — `/request` attempt-record (every request that passes rate-limit) | INSERT one row at `NOW()` keyed by `email_hash` | No effect | INSERT one row |
| Step 3 — `/request` user lookup | `find_by_email` (the first DB read that *could* differ between the two paths) | No effect | No effect |
| Step 4a — `/request` (unknown email) | Constant-time padding, return Ok | No change | (already recorded in step 2) |
| Step 4b — `/request` (known email) | 32 random bytes → URL-safe base64 → SHA-256 token hash → INSERT into `magic_link_tokens` (deleting any prior PasswordReset row first); send email | Delete prior PasswordReset row(s); insert new row | (already recorded in step 2) |
| Validation — `/validate` | Hash incoming raw token, lookup by hash, check `exp > now`, check `purpose = PasswordReset` | No mutation | No mutation |
| Consumption — `/complete` | Same checks as validation, then `DELETE` all of user's `PasswordReset` tokens + `UPDATE users.password` in one transaction | Row removed; password updated atomically | No change — attempts are kept; deleting them would let an attacker mask their tracks by completing a reset |
| Expiry (passive) | 30 minutes from issuance (`PASSWORD_RESET_TOKEN_EXPIRY_SECONDS=1800`) | Expired rows remain until next issuance or admin cleanup; queries filter on `exp > now` | N/A — attempts have no TTL semantic |
| Ops sweep (active) | `domain::password_reset::sweep_old_attempts(db, retention_days)` | No effect | DELETE rows where `attempted_at < NOW() - retention_days` |

### Why record-first

The audit row is inserted **before** the user lookup (and before token creation on the known-user branch), not after. Three reasons:

1. **Enumeration safety.** Recording before the user lookup keeps the unknown-email and known-user paths observationally identical up to the point the rate-limit could fire. See [Enumeration Safety on Both Paths](#enumeration-safety-on-both-paths) for the bug this prevents.
2. **Consistency under failure.** If the rate-limit check passes but token issuance fails (DB transient error), the next request — moments later — must still see the previous attempt and apply rate-limiting. Recording first guarantees this.
3. **Conceptual correctness.** "Attempt" means "user tried to trigger a reset," not "we succeeded." A user who triggers 5 requests that all fail mid-issuance has still used the system 5 times in the rate-limit's eyes.

The cost is that a transient DB error during issuance burns one of the email's 5 daily attempts — slightly user-hostile but the right default for a security mechanism.

## Security Decisions

Every decision below was made deliberately to defeat a specific class of attack. See [Threat Model](#threat-model) for the scenarios these mitigations defend against.

### Enumeration Safety

| Mechanism | Defends Against |
|---|---|
| `POST /password-reset/request` always returns 200 regardless of email existence | Status-code-based user enumeration on the first request |
| **Hard signal-ceiling on response timing**: all path-distinguishing work runs in a background `tokio::spawn`, then the sync path pads to a fixed target (150 ms). Sync ops key on `email_hash`, never on the user record. See [Hard Signal-Ceiling on Response Timing](#hard-signal-ceiling-on-response-timing). | Timing-based user enumeration via response-latency oracle — including the previously-undetected MailerSend-dominated bimodal distribution (PR #311 review catch). |
| **Rate limit fires uniformly on both paths via email-hash key, checked BEFORE user lookup** — see [Enumeration Safety on Both Paths](#enumeration-safety-on-both-paths) | Status-code-based user enumeration via 200/429 split on subsequent requests (PR #311 review catch) |
| `POST /password-reset/validate` returns only `first_name` + `last_name` on success | PII over-exposure if a token is ever guessed/leaked |
| Collapsed `400 invalid_or_expired_token` (no distinction between unknown / expired / wrong-purpose) | Status-code-based token-state oracle |

### Token Strength & Protection

| Mechanism | Defends Against |
|---|---|
| 256 bits of entropy per token (32 random bytes from `rand::thread_rng()`) | Brute-force guessing (search space ≈ 10⁷⁷) |
| SHA-256 hash stored in DB, never the raw token | DB-leak token harvesting |
| Single-use: token deleted atomically with password update | Replay after legitimate use |
| 30-minute TTL (vs 72h for setup tokens) | Bounded exposure if email is intercepted or phished |
| Path-segment URL format (`/reset-password/<token>`) | Token leakage via HTTP `Referer` and query-string-aware logs |
| Per-email rate limit (1/60s, 5/24h) backed by a **separate append-only `password_reset_attempts` audit table** → `429 password_reset_rate_limited` | Inbox-flooding harassment and brute-force probing |

### Purpose Separation

The `magic_link_tokens.purpose` column (enum `Setup` | `PasswordReset`) ensures token usage is scoped to its intended flow:

- `find_by_token_hash` filters on `purpose = ?` so a leaked Setup token cannot be redeemed at `/password-reset/complete`, and vice versa.
- `delete_all_for_user` is purpose-scoped — issuing a reset token does NOT invalidate a pending Setup token. The two flows operate on disjoint token sets for the same user.

Existing tokens in the table at migration time are backfilled to `Setup`. The column is `NOT NULL` with no default — every new insertion must explicitly declare its purpose, preventing accidental defaulting.

### Hard Signal-Ceiling on Response Timing

Response time to `POST /password-reset/request` is bounded to **approximately 150 ms** regardless of whether the email maps to a real user, with any unavoidable variance (DB jitter, scheduler noise) being **identical across both branches**. Timing carries no enumeration signal.

**The bug this prevents (PR #311 review catch).** An earlier design used a 75 ms sleep on the unknown-email branch to "match the success-path duration." That sleep was inadequate by an order of magnitude:

| Branch | Sync work | Time |
|---|---|---|
| Unknown email | DB lookup + 75 ms sleep | ~80–150 ms |
| Known user | DB lookup + DB write × 2 + **synchronous HTTPS POST to MailerSend** | ~300–700 ms+ (HTTPS dominates) |

The MailerSend round-trip alone (100–500 ms typical, can spike to 1–2 s) made the two distributions trivially distinguishable with a handful of `time curl` samples. The "constant-time padding" was decorative.

**The fix: spawn-then-pad.** Two structural changes work together:

1. **Move all path-distinguishing work into a `tokio::spawn` background task.** The user lookup, token issuance, and email send no longer affect response timing. The handler's sync path becomes:

   ```
   hash_email   →   rate-limit check   →   record attempt   →   spawn background   →   pad to 150ms   →   return
   ```

   The first three steps key on `email_hash`, never on the user record. Their timing is the same regardless of email-existence.

2. **Pad to a fixed target duration on the sync path.** `Instant::now()` at handler entry; `tokio::sleep(target - elapsed)` before returning. The 150 ms target safely overshoots the typical sync-path duration (~15–40 ms in production) while staying well below user-perceptible latency.

**Why this is a "signal-ceiling," not a literal time-ceiling.** A literal hard time-cap (e.g. "always return at exactly 150 ms regardless of what's happening") isn't achievable with synchronous DB calls: if Postgres has a slow-query spike, the response physically can't return until the sync DB ops complete (without breaking data consistency). What this design achieves instead is **timing variance that doesn't differ between branches**: a slow DB affects both branches equally, so the overshoot from a 2-second DB hiccup tells an attacker nothing about email existence.

**API semantic change vs purely synchronous designs.** "200 OK" now means "we accepted your reset request" rather than "we processed it." Token issuance and email send happen after the response returns. This is acceptable because:

- Email send was already best-effort (the synchronous design also returned 200 even on email-send failure).
- The wire contract (`PasswordResetEndpoints` v1) is committed to enumeration-safe always-200, which precludes surfacing user-specific errors anyway.
- The user has no in-protocol way to observe whether the work succeeded (they check their inbox).

**Failure handling on the background task.** All errors are logged at WARN and discarded — by the time the spawn runs, the client has the `200 OK`. Failure modes: unknown email (logged, no further action), DB error during user lookup or token issuance (logged), email-send failure (logged; the token still exists until expiry, the user can request another). The audit-trail WARN log "[password-reset] reset link issued for user X" still fires, just from the spawned task rather than the handler — same observability, slightly delayed.

**Why moving the email send was the load-bearing fix.** A naive defense would just pad to a target that overshoots MailerSend (e.g. 750 ms). But: (a) every legitimate user would then wait 750 ms for the success response, (b) the pad would still need re-tuning if MailerSend ever got slower or faster, and (c) it would still leak DB-variance differences between branches. Moving the variable-cost operations off the response path removes the root cause; padding handles the residual.

### Enumeration Safety on Both Paths

A response-code split between the unknown-email and known-user paths is itself an enumeration oracle — even if the *first* response is identical (always 200), any subsequent state-dependent behavior that differs between the two paths leaks existence.

**The bug this prevents (PR #311 review catch):** An earlier version of `request_password_reset` ran the rate-limit check *after* the user lookup, only on the known-user path. The 2nd request from an attacker within 60 seconds:

| Email | 1st response | 2nd response (within 60s) |
|---|---|---|
| Unknown to the system | 200 | **200** (no rate limit was checked) |
| Known to the system | 200 | **429** (rate-limit fired on the prior recorded attempt) |

The 2nd response was 100% deterministically distinguishable, defeating the always-200 guarantee on the 1st response.

**The fix.** Three structural changes that together make the response observationally identical on both paths:

1. **Rate-limit by email-hash, not user_id.** `hash_email(email)` normalizes (lowercase + trim) and SHA-256-hashes the address. This becomes the rate-limit key, independent of whether a user record exists. (Capitalization variants of the same address share a rate-limit bucket — without normalization, an attacker could enumerate via `Foo@example.com` vs `foo@example.com`.)

2. **Check rate limit BEFORE the user lookup.** The order in `request_password_reset` is now: `hash_email → enforce_rate_limit → record_attempt → find_by_email → (branch by user existence)`. Any rate-limit rejection fires identically before the user lookup runs.

3. **Record attempts for unknown emails too.** The audit row is inserted on every request that passes the rate-limit check, regardless of whether the email matches a user. This keeps both paths observationally indistinguishable in their DB side effects up to step 3.

**Trade-off accepted.** An attacker probing many unknown emails grows the `password_reset_attempts` table faster than if we only recorded known users. Bounded by the rate limit itself (5 attempts/email/24h), the worst-case growth is 5 × N_probed_emails per day. The sweep job ([Ops Maintenance](#ops-maintenance) below) handles long-term retention. A legitimate user whose email was previously probed by an attacker can still reset their password after the attacker's rate-limit budget for that email expires — bounded 24-hour DOS, same as the standard rate-limit trade-off.

**Hash properties.** Plain SHA-256 of the normalized email (no pepper). The hash provides modest defense-in-depth — a DB leak doesn't expose plaintext emails directly — but is not reversibility-resistant: an attacker with a candidate email list can determine which ones have attempted resets. Adding a server-side pepper would harden this; deferred unless we accumulate sensitive forensic patterns in the table.

### Rate-Limit Audit Separate from Token State

The per-email rate limit (1/60s, 5/24h) is enforced by **counting rows in a dedicated `password_reset_attempts` audit table**, NOT by counting rows in `magic_link_tokens`. The two tables serve fundamentally different roles:

| Concern | Table | Key | Cardinality semantics |
|---|---|---|---|
| What's the current redeemable token? | `magic_link_tokens` (with `purpose=PasswordReset`) | `user_id` (FK to users) | **State table**: at most one live row per `(user_id, purpose)`. `create_magic_link` runs `delete_all_for_user` before insert. |
| How many requests have been made for this email recently? | `password_reset_attempts` | `email_hash` (SHA-256 of normalized email; no FK) | **Audit table**: append-only. One row per request — recorded for both known and unknown emails. Pruned by an out-of-band ops sweep, never by request-path code. |

**Why this matters.** An earlier design conflated these two roles by counting rows in `magic_link_tokens` for the daily-cap check. That made the cap **mathematically unreachable**: because the state-table semantics required deleting the prior row on every issuance, the count returned 0 or 1, never ≥ 5. An attacker honoring only the 60-second min-interval could emit ~1,416 reset emails per day instead of the intended 5. The bug was caught in PR #311 review.

**Defenses this design enables that the conflated version did not:**

| Defense | How the audit table provides it |
|---|---|
| Daily-cap rate limit (5/24h → `429`) actually fires | Row count over a 24h window is meaningful because rows accumulate |
| Forensic trail when users report abuse ("someone kept trying to reset my account last week") | Attempts persist beyond token consumption / deletion |
| Post-consumption rate-limit accuracy | Successful resets delete the token but **keep** the attempts; the next reset request after a completion still sees its full attempt history |
| Future per-IP rate limiting (deferred, see Out of Scope) | The attempts table can grow an `ip_address INET` column without touching the token table |

**Trade-offs:**

- The attempts table grows unbounded without intervention. Mitigated by the `sweep_old_attempts(db, retention_days)` ops function (see [Ops Maintenance](#ops-maintenance) below). At the rate-limit cap (5/email/day) the maximum growth per email is ~1,825 rows/year — negligible for any realistic user base, even allowing for attacker probes of unrelated emails.
- An "attempt" is recorded *before* token issuance succeeds (and before the user lookup). A transient DB error during issuance burns one of the email's 5 daily attempts. This is the right default for a security mechanism (better to over-count attempts than under-count), and is documented inline in `request_password_reset`.

**Design principle.** Any time a single table appears to serve both as a "current state" projection and an "audit log" of past events, the seam between those two semantics is bug-prone. Split them. This same pattern applies to future similar features (e.g. if we add rate-limited login retries or 2FA challenges).

### Password Policy

Server-side password validation enforced in `domain::password_policy::validate_password`, applied at both `POST /password-reset/complete` and `POST /magic-link/complete-setup` (the two endpoints that set a user's password). Independent of any FE validation — defense in depth.

| Rule | Value | Source / rationale |
|---|---|---|
| Non-empty after `trim()` | required | A literal `""` would otherwise argon2-hash and commit. Catches accidental whitespace-only submissions and the bug an earlier review caught. |
| Minimum length | **12 characters** (Unicode scalar values, not bytes) | NIST 800-63B requires ≥8; 12 is the modern industry baseline. Raises offline-brute-force cost while remaining typeable. |
| Maximum length | **128 characters** | Prevents argon2 hashing DoS on pathologically long inputs. Well above any realistic password-manager output. |
| Character-class complexity (uppercase / digit / symbol) | **NOT enforced** | NIST 800-63B explicitly *recommends against* these — they push users toward predictable patterns (`Password1!`) that reduce real-world entropy. Length is the load-bearing dimension. |

Constants `MIN_PASSWORD_LENGTH` and `MAX_PASSWORD_LENGTH` are public in [`domain/src/password_policy.rs`](../../domain/src/password_policy.rs) so the FE coordination doc (`password_policy` decision on the coordinator blackboard) can reference them. If the policy changes, those consts move and a new version of the blackboard decision will be posted.

Frontend should mirror the policy for instant user feedback. Server enforcement is the security boundary; client enforcement is the UX redundancy.

### `/validate` is Non-Destructive — Trade-off Acknowledged

The `POST /password-reset/validate` endpoint **does not consume the token**. A successful validate returns the user's sanitized profile (first_name, last_name) and leaves the token in the DB, valid for further calls until it expires or is consumed by `/complete`.

**Why non-destructive.** The FE state machine needs to validate the token before the user submits the form — otherwise it can't render a personalized "Hi Alice, set your new password" page. A user might take 30 seconds or 5 minutes between clicking the email link (validate fires) and submitting the form (complete fires). Consuming the token on first validate would force a tight time window between those two events that breaks realistic user flows.

**The trade-off this creates.** An attacker who obtains a leaked but live token (browser history, shared device, accidental forward) can call `/validate` repeatedly to extract first/last name without consuming the token. The legitimate user, meanwhile, can still complete their reset via `/complete` because the token stays valid.

**Why we accept this.** The bounded-harm analysis:

| Threat | Bounded by |
|---|---|
| Attacker extracts first/last name | First/last name is not high-secrecy PII — LinkedIn, the org's coachee list, and the user's email signature already expose it for most users |
| Attacker uses the token to complete the reset themselves | Single-use enforcement on `/complete` — once anyone completes, all tokens for the user are deleted. The user discovers via "I clicked my reset link and got an error" and re-requests. |
| Attacker keeps the token live indefinitely | 30-minute TTL bounds the exposure window |
| Attacker spams `/validate` to enumerate / load-test the DB | Per-IP throttle (~10 req/min) + wrong-length-token rejection at 400 cut the DoS surface |

**A consume-on-first-validate-window design** (e.g. delete the token 60s after first validate) was considered and deferred. It would tighten the leaked-token threat at the cost of a hard FE UX cliff. Revisit if the threat model changes (e.g. if first/last name becomes more sensitive in some org context).

### Token Transport: Body, Not Query String

All three password-reset endpoints (`/request`, `/validate`, `/complete`) accept their sensitive payload (email or token) in the **JSON request body**, never in URL query parameters.

**Why this matters.** Query-string values land in places body values don't:

- Web-server access logs (axum/tower-http `TraceLayer`, nginx, Caddy default config)
- Reverse-proxy / CDN access logs (any ingress between the client and us)
- Browser `window.history` and DevTools network panel
- Error-reporting and APM integrations that capture request URLs
- HTTP `Referer` headers when a token-bearing page links anywhere else

Token-in-body bypasses every one of those channels in a single change. This applies in **both directions of the token's travel**: the email link uses a path segment (not query string), and the FE→BE validation call uses a body (not query string). Same principle, both edges.

**This is the difference between `PasswordResetEndpoints` v1 and v1.1.** The v1 contract specified `GET /password-reset/validate?token=<raw>` — that was wrong. v1.1 corrects this to `POST /password-reset/validate` with `{ "token": "..." }` in the body. The FE caught this during PR review of their client implementation (see `password_reset_validate_token_transport` on the coordinator blackboard).

**Why POST is the right verb here despite "validate" being read-like.** REST purity would lean toward GET-with-header for non-mutating reads, but: (a) `/request` and `/complete` already use POST in the same endpoint group — staying consistent across the three endpoints is a UX win for FE devs; (b) `Authorization: Bearer <token>` is the alternative header transport but is unusual for a non-account-bound short-lived token; (c) the cost of "wrong verb for the semantic" is small, the cost of leaking tokens to log streams is real.

### Input Validation at the HTTP Boundary

Length and shape validation runs **before** any handler logic — the cheapest DoS amplifier on an unauthenticated endpoint is a pathologically large input field (e.g. a 10 MB email would still trigger SHA-256 hashing + DB query). These checks cut those attack vectors before any expensive work runs.

| Field | Limit | Source | Failure response |
|---|---|---|---|
| `email` (POST /request) | Non-empty, length ≤ 254 octets, contains `@` | RFC 5321 caps deliverable email addresses at 254 octets in practice | `400 Bad Request` |
| `token` (POST /validate body, POST /complete body) | Length == 43 | Tokens we issue are exactly 32 random bytes encoded as URL-safe base64 without padding = always 43 chars; any other length is impossible for a real token | `400 Bad Request` |
| `password` (POST /complete, POST /magic-link/complete-setup) | 12 ≤ length ≤ 128 (after `trim()`) | See [`domain::password_policy`](../../domain/src/password_policy.rs) and the `password_policy` decision on the coordinator blackboard | `422 validation_error` |

Validators live at the web boundary in [`web::params::validation`](../../web/src/params/validation.rs); the password policy lives in the domain layer because it's a business rule, not an HTTP shape concern. Both layers enforce independently of any FE validation.

### TOCTOU-Free Rate-Limit Check via Advisory Lock

The rate-limit check (read `find_most_recent` + `count_since`) and the attempt-record write (INSERT into `password_reset_attempts`) run inside a **single transaction** that holds a PostgreSQL **advisory lock keyed on the email hash**. Without this serialization, two concurrent requests for the same email could both pass the rate-limit check (each reading a snapshot with no prior recent attempts) before either has written its attempt row, then both insert and both fire emails — a "two-burst" inbox flood every 60 seconds.

**Lock pattern.** `entity_api::password_reset_attempt::lock_email_hash(&txn, &email_hash)` issues `SELECT pg_advisory_xact_lock(hashtext($1)::bigint)`. The lock:

- Is held for the lifetime of the transaction (auto-released on commit or rollback).
- Serializes concurrent requests for the **same** `email_hash` — they queue behind each other one at a time.
- Does NOT serialize requests for **different** `email_hash` values — `hashtext()` distributes hashes across the 64-bit lock-key space, so different emails take different lock IDs and proceed in parallel.

A `hashtext()` collision (two unrelated email_hashes hashing to the same int64) would cause those two emails' requests to serialize unnecessarily — a minor performance impact, not a correctness bug. The 64-bit space makes such collisions astronomically rare.

**Why advisory lock, not `SELECT … FOR UPDATE` or SERIALIZABLE.** There's no "row" to lock at rate-limit-check time — the rate-limit query reads from a set of rows (possibly empty) and there's nothing to take a row lock on. SERIALIZABLE isolation would work, but its retry-on-conflict semantics would require us to wrap call sites in retry loops; the advisory lock is more surgical. PostgreSQL-specific, documented in `entity_api::password_reset_attempt::lock_email_hash`.

**Operation order inside the transaction:**

```
db.begin()
  → lock_email_hash(email_hash)          [SELECT pg_advisory_xact_lock]
  → enforce_rate_limit(email_hash)       [find_most_recent, count_since]
  → record_attempt(email_hash)           [INSERT]
db.commit()
```

The lock is held continuously across all four operations. A second concurrent request for the same email blocks at `lock_email_hash` until the first transaction commits — by then, the second one's `find_most_recent` will see the first's insert and rate-limit correctly.

**Cost.** One extra DB roundtrip per request (the lock SELECT). Sub-millisecond on the connection-pooled PG that backs production. Worth it.

### Session Invalidation on Password Change

When a user resets (or initially sets) their password, **all of their existing sessions are invalidated automatically on the next request**. The mechanism is `axum_login`'s built-in session-auth-hash check, not an active session-store sweep.

[entity/src/users.rs:76-89](../../entity/src/users.rs#L76-L89):

```rust
impl AuthUser for Model {
    type Id = crate::Id;
    fn id(&self) -> Self::Id { self.id }
    fn session_auth_hash(&self) -> &[u8] {
        self.password
            .as_deref()
            .map(|p| p.as_bytes())
            .unwrap_or_else(|| self.id.as_bytes())
    }
}
```

`axum_login` captures `session_auth_hash()` at login time and stores it in the session. On every subsequent authenticated request, it re-fetches the user from the DB (by session-stored user_id), recomputes `session_auth_hash()` against the *current* user record, and compares to the stored hash. **Mismatch → session is treated as expired → 401.**

Since our `session_auth_hash()` returns the password hash bytes:

| Event | Effect on `session_auth_hash()` | Effect on existing sessions |
|---|---|---|
| Password reset via `/password-reset/complete` | Old `argon2(old_pw)` → new `argon2(new_pw)`, different bytes | All existing sessions invalidated on next request |
| Initial password set via `/magic-link/complete-setup` | `id_as_bytes` → `argon2(new_pw)`, different bytes | All existing sessions invalidated (in practice no real sessions exist yet for a brand-new user) |
| Any other user update (email, name, etc.) | Password bytes unchanged | Sessions unaffected — by design, profile edits don't sign the user out |

**The primary use case for password reset works.** "I think my account is compromised; reset and lock the attacker out." After the reset, the attacker's session hash no longer matches the user's current hash, and their next authenticated request returns 401. The FE's `SessionCleanupProvider` handles the 401 by clearing client-side state and redirecting to login.

**Behavior across deploys.** The session store is `tower_sessions` (currently in-memory for our deploys). On a backend restart, all sessions are dropped anyway. *Between* restarts (potentially weeks during normal uptime), the `session_auth_hash` mismatch is what protects compromised accounts — not the restart cadence.

**Why this is safer than active session iteration.** An active "iterate the session store, find sessions belonging to user X, delete them" mechanism is what some auth systems implement, and it works — but it requires the session store to support that operation, doesn't generalize cleanly across storage backends, and has subtle race conditions (a request issued mid-iteration might survive). The `session_auth_hash` approach is checked on *every authenticated request* against the *current user record*, so it's both backend-agnostic and atomic.

### Logging Hygiene & Security Audit Trail

The password-reset code path emits a **WARN-level audit trail** so security/ops operators can grep `[password-reset]` and see every interesting event without enabling DEBUG. **Raw emails and raw tokens are never logged at any level** — when correlation is needed, we log a hash-prefix.

| Event | Level | Where | Contains |
|---|---|---|---|
| Endpoint hit — `/request`, `/validate`, `/complete` | WARN | `web/src/controller/password_reset_controller.rs` | endpoint name only |
| Reset requested for unknown email | WARN | `domain::password_reset` | `email_hash_prefix` (first 16 hex chars of SHA-256 of normalized email) — gives operators a correlation handle without exposing plaintext |
| Reset link issued for known user | WARN | `domain::password_reset` | `user_id` UUID |
| Password mismatch on `/complete` | WARN | `domain::password_reset` | no PII |
| Rate-limit hit (min-interval or daily cap) | WARN | `domain::password_reset` | no PII (rate-limit fires before user is known) |
| Email-send failure | WARN | `domain::password_reset` | `user_id` UUID + error |
| Password successfully reset (password changed) | WARN | `domain::password_reset` | `user_id` UUID |
| Token validation (underlying) — not found / expired | WARN | `domain::magic_link_token` | purpose discriminator |

**General rules across the path:**

- Raw email addresses are **never logged at any level**. Even DEBUG is unsafe: ops teams enable DEBUG during incidents, log aggregators may retain DEBUG longer than WARN, and access boundaries are coarser than "by log level." When we need correlation, we log the first 16 hex chars of `hash_email(email)` as an `email_hash_prefix=` field — operators can match the same email across log lines without ever seeing the plaintext.
- The raw token is **never** logged at any level. Only the SHA-256 hash (already stored in the DB) may be logged, and even that is reserved for ERROR-level audit traces if needed.
- Every WARN message uses the `[password-reset]` prefix so operators can `grep` the entire flow with one search.

### Layered Rate Limiting: Per-IP and Per-Email

The endpoints are defended by **two complementary rate limits** working at different scopes. They are not substitutable — each closes attacks the other can't:

| Layer | Scope | Defends | Doesn't defend |
|---|---|---|---|
| Per-IP throttle ([`tower_governor`](../../web/src/middleware/throttle.rs)) | Per client IP across all 3 endpoints — `AUTH_ENDPOINT` policy: ~10 req/min, burst 10 | Mass scanning (one attacker varying emails or tokens per request) | Targeted abuse from many IPs (botnet) |
| Per-email DB rate limit (`password_reset_attempts`) | Per email-hash, scoped to `/request` only — 1/60s + 5/24h | Targeted Alice-flooding (one attacker repeatedly hitting Alice's email) | Mass enumeration (different email per request) |

**Why both are required.** An earlier PR-review iteration treated these as comparable defenses ("v1 ships with per-email DB-based limit only; per-IP deferred to a follow-up"). That was wrong: an attacker varying emails per request never trips a per-email limit, no matter how strict. Per-email limits defend **one specific email**; per-IP limits defend **the endpoint surface as a whole**. Either alone leaves a wide gap.

The 429 response shapes also differ — see the [Layered 429 Responses](#layered-429-responses-shape) note below.

**Trust assumption** (briefly): the per-IP layer uses `SmartIpKeyExtractor`, which trusts `X-Forwarded-For` from our nginx reverse proxy. Without nginx in front, headers are spoofable and the throttle is defeated. See [`throttling.md`](throttling.md) for the full trust model and what happens in local-dev vs production environments.

#### Layered 429 responses (shape)

| Trigger | Status | Body |
|---|---|---|
| Per-IP throttle (outer ring) | `429 Too Many Requests` | Plain text `Too Many Requests` + `Retry-After` header |
| Per-email rate limit (inner ring, only on `/request`) | `429 Too Many Requests` | JSON `{ status_code: 429, error: "password_reset_rate_limited", message: "..." }` |

Both are 429; the body differs. The FE should handle 429 generically (show "you're rate-limited, please wait") without depending on the body shape — it sees whichever fired first. Documented on the wire-format contract.

### Authorization Model

| Endpoint | Authorization Signal |
|---|---|
| `POST /password-reset/request` | None — by design. Submitting any email is harmless because the token never goes to the requester. |
| `POST /password-reset/validate` | Possession of a valid `PasswordReset` token (transmitted in JSON body). |
| `POST /password-reset/complete` | Possession of a valid `PasswordReset` token. |
| `PUT /users/:id/password` (pre-existing, distinct endpoint) | `authenticated_user.id == user_id` ([protect/users/passwords.rs:21](../../web/src/protect/users/passwords.rs#L21)) |

The unauthenticated reset endpoints do **not** weaken the authenticated `PUT /users/:id/password` model — they are an additive credential-recovery channel, not a replacement.

## Threat Model

| Scenario | Mitigation |
|---|---|
| Mallory submits Alice's email to `/password-reset/request` to take over her account | Reset link is delivered to Alice's inbox, not Mallory. Mallory has no path to the token. Alice's current password is unchanged. |
| Mallory floods Alice's inbox with reset emails (one email, many requests) | Per-email rate limit caps issuance at 1/60s, 5/24h → `429`. Enforced via the `password_reset_attempts` audit table — see [Rate-Limit Audit Separate from Token State](#rate-limit-audit-separate-from-token-state) for the design rationale and the bug this separation prevents. |
| Mallory mass-scans `/password-reset/request` with millions of emails to enumerate the user base or burn MailerSend cost | Per-IP throttle on the endpoint group (`AUTH_ENDPOINT` policy: ~10 req/min, burst 10) limits one attacker to a few hundred attempts per day, far below useful enumeration rate. See [Layered Rate Limiting: Per-IP and Per-Email](#layered-rate-limiting-per-ip-and-per-email). |
| Mallory spams `POST /password-reset/validate` with random tokens (in body) to brute-force or strain the DB | Per-IP throttle applies to `/validate` and `/complete` the same way it does to `/request` — single attacker can't issue more than 10 req/min against the endpoint group. (256-bit token entropy means even an unlimited attacker couldn't guess; the throttle defends DB load.) |
| Mallory probes the request endpoint **once** per email to map which emails have accounts | Always-200 response + hard signal-ceiling on response timing (sync path is identical across branches; path-distinguishing work runs in a background task). See [Hard Signal-Ceiling on Response Timing](#hard-signal-ceiling-on-response-timing). |
| Mallory times responses against known and unknown emails looking for a bimodal distribution from the MailerSend HTTPS round-trip | MailerSend send runs in a background `tokio::spawn` after the response is sent. Known-path response timing no longer depends on email-provider latency. |
| Mallory probes the request endpoint **twice in quick succession** with the same email — looking for a 200/429 split to leak existence | Rate-limit fires uniformly on both paths via email-hash key checked before user lookup — both unknown emails and known users get 429 on the 2nd request inside the min-interval window. See [Enumeration Safety on Both Paths](#enumeration-safety-on-both-paths). |
| Mallory probes with capitalization or whitespace variants (`Alice@x.com` vs `alice@x.com`) to bypass per-email rate limit | Email is normalized (lowercase + trim) before hashing, so all variants share a single rate-limit bucket. |
| Mallory brute-forces `/password-reset/complete` with random tokens | 256-bit entropy + 30-minute TTL make guessing computationally infeasible. |
| Mallory reuses a stolen setup-email token at `/password-reset/complete` | Purpose-scoped lookup rejects tokens with `purpose ≠ PasswordReset`. |
| Mallory tries to replay a previously-consumed reset link | Token is deleted atomically with the password update; subsequent attempts return `400 invalid_or_expired_token`. |
| Mallory intercepts the email in transit | TLS protects SMTP transit. Out-of-scope at the application layer. |
| Mallory has compromised Alice's email account | Out of scope: at this point Mallory controls account recovery for every service Alice uses. |
| Authenticated user A tries to reset user B's password via the existing change-password endpoint | `protect::users::passwords::update_password` middleware enforces `authenticated_user.id == user_id`. Pre-existing, unchanged. |
| Token leaks via HTTP `Referer` when the FE reset page loads a third-party resource | Path-segment format prevents query-string-style leakage. FE additionally sets `Referrer-Policy: same-origin` on token-bearing pages (tracked separately on the coordinator blackboard). |
| Mallory holds a stolen session cookie for Alice's account; Alice resets her password to lock him out | After the reset, `users.password` holds a new argon2 hash. On Mallory's next authenticated request, `axum_login` recomputes `session_auth_hash()` against the current user record (new password bytes), compares to the session-stored hash (old password bytes), sees a mismatch, and returns 401. See [Session Invalidation on Password Change](#session-invalidation-on-password-change). |
| Mallory spams `/password-reset/validate` with random tokens hoping to extract user data, DoS the DB, or amplify a future log-leak attack | Per-IP throttle on the `password-reset` route group applies to `/validate` identically to `/request` and `/complete` — `AUTH_ENDPOINT` policy (~10 req/min per IP, burst 10). With 256-bit token entropy, brute-force is computationally infeasible regardless; the throttle defends DB load. Wrong-length tokens are rejected with 400 at the HTTP boundary before any DB work — see [Input Validation at the HTTP Boundary](#input-validation-at-the-http-boundary). |
| Mallory obtains a leaked but live token (browser history, shared device, email forward) and calls `/validate` repeatedly to extract Alice's first/last name | `/validate` is intentionally non-destructive — consuming the token on validate would force the FE to call `/complete` within a tight window of `/validate`, breaking realistic UX (a user might take >60s between clicking the email link and submitting the form). The exposure is bounded: (1) tokens TTL out in 30 minutes; (2) first/last name is not high-secrecy PII (LinkedIn, org coachee lists, email signatures already expose it); (3) the more consequential threat (Mallory actually completing the reset) is mitigated by single-use enforcement on `/complete`, the 30-min TTL, and email-delivery to the rightful owner. See "non-destructive validate" note in the [Hard Signal-Ceiling](#hard-signal-ceiling-on-response-timing) section. |

## Out of Scope for v1

Each deferral is a deliberate scoping decision, not an oversight.

| Item | Reason for Deferral |
|---|---|
| **Per-IP rate limiting at the application layer is shipped in v1** (was originally deferred) | See [`throttling.md`](throttling.md) for the design. What remains out of scope: per-AS / per-network-owner throttling and reputation-based blocking — those defend distributed-botnet attacks and belong at the CDN/WAF edge, not the application layer. Add when we ever sit behind Cloudflare or similar. |
| Periodic sweep of expired tokens | Expired tokens in `magic_link_tokens` are validated as inert (`validate_token` refuses them) but are not auto-deleted. Three implicit cleanup paths cover the common case: (a) issuing a new token of the same purpose runs `delete_all_for_user(user_id, purpose)` first; (b) user deletion cascades via the `user_id` FK; (c) successful consumption deletes inside the consume transaction. A separate periodic sweep would only matter at scale or under a regulatory deletion requirement — neither applies today. **Add a sweep migration when**: (i) the table exceeds ~10⁶ rows, (ii) `EXPLAIN` shows index degradation on the `token_hash` UNIQUE index, or (iii) a compliance requirement mandates prompt deletion. |

## Monitoring & Operational Visibility

The WARN-level audit trail (see [Logging Hygiene & Security Audit Trail](#logging-hygiene--security-audit-trail) above) is the substrate for both ad-hoc operator triage and a dedicated Grafana monitoring panel. Every monitoring recipe in this section builds on the same `[password-reset]` log prefix — no new instrumentation is required for v1.

### Ad-hoc Operator Recipes (`journalctl`)

Use these for incident triage or quick spot-checks from a shell on the backend host.

```bash
# What's happening on the reset endpoints right now?
journalctl -u refactor-platform -f | grep '\[password-reset\]'

# Audit: who got reset emails in the last hour?
journalctl --since '1 hour ago' | grep '\[password-reset\] reset link issued for user'

# Abuse signal: anyone hitting the rate limit?
journalctl --since today | grep '\[password-reset\] rate-limited'

# Enumeration probes: requests for unknown emails over the last 24h
journalctl --since '24 hours ago' | grep '\[password-reset\] reset requested for unknown email' | wc -l

# Operational health: email-send failures
journalctl --since '6 hours ago' | grep '\[password-reset\] failed to send email'

# Forensic: which unknown-email hashes are being probed? Raw emails are
# never logged — correlate by the email_hash_prefix= field instead.
# Same prefix across multiple lines = same email being probed.
journalctl --since '15 minutes ago' | grep '\[password-reset\] reset requested for unknown email' \
  | grep -oE 'email_hash_prefix=[0-9a-f]+' | sort | uniq -c | sort -rn
```

### Grafana Panel Setup (Loki + LogQL)

A single Grafana dashboard — call it **"Password Reset (Security)"** — should host the panels below. All queries assume logs are shipped to Loki with `app="refactor-platform"` as a label; adjust the selector to match your shipping setup.

| Panel | Type | LogQL query | Why it matters |
|---|---|---|---|
| **Endpoint activity** (stacked rate) | Time-series, stacked | `sum by (endpoint) (rate({app="refactor-platform"} \|~ "\\[password-reset\\] /(?P<endpoint>request\|validate\|complete) endpoint hit" [5m]))` | Shows traffic shape across the three endpoints. Baseline is near-zero; spikes warrant investigation. |
| **Reset links issued** | Time-series + single-stat | `sum(rate({app="refactor-platform"} \|~ "\\[password-reset\\] reset link issued for user" [5m]))` | Direct count of successful issuances. Should correlate with email-send success below. |
| **Passwords actually changed** | Time-series + single-stat | `sum(rate({app="refactor-platform"} \|~ "\\[password-reset\\] .* completed password reset" [5m]))` | Most consequential event — a password actually changed. Compare against "links issued" for drop-off rate. |
| **Rate-limit triggers** ⚠️ | Time-series, alert candidate | `sum by (kind) (rate({app="refactor-platform"} \|~ "\\[password-reset\\] rate-limited \\((?P<kind>min-interval\|daily-cap)\\)" [5m]))` | Healthy baseline = zero. Sustained non-zero is the abuse signal. Split by `min-interval` vs `daily-cap` to distinguish rapid-fire from slow-and-steady. |
| **Unknown-email probes** ⚠️ | Time-series, alert candidate | `sum(rate({app="refactor-platform"} \|~ "\\[password-reset\\] reset requested for unknown email" [5m]))` | Healthy baseline = near-zero (legitimate typos). Sustained elevated rate = enumeration probe; pair with IP-level inspection. |
| **Email-send failures** | Time-series, alert candidate | `sum(rate({app="refactor-platform"} \|~ "\\[password-reset\\] failed to send email" [5m]))` | Operational health — distinguishes "MailerSend is down" from "our code is broken." |
| **Password-mismatch errors** | Time-series | `sum(rate({app="refactor-platform"} \|~ "\\[password-reset\\] password confirmation mismatch" [5m]))` | Mostly user error (typos in the form). A persistent spike for a single user could indicate phishing attempt with stolen-link replay against a real user. |
| **Issuance → completion ratio** (gauge) | Stat / gauge | `sum(count_over_time({app="refactor-platform"} \|~ "completed password reset" [24h])) / sum(count_over_time({app="refactor-platform"} \|~ "reset link issued for user" [24h]))` | The healthy ratio is roughly 0.6–0.9 (some users start the flow and never finish). Sustained <0.2 = bogus reset emails being sent (phishing campaign targeting your users); sustained >1.0 is impossible and indicates a parser bug. |
| **Recent audit log** (table) | Logs | `{app="refactor-platform"} \|~ "\\[password-reset\\]" \| line_format "{{.timestamp}} {{.message}}"` | Raw scrolling audit table for the most recent N events. |

#### Suggested alert rules

Each is paired with the panel it sits on. Set in Grafana Alerting or whatever the platform uses.

| Alert | Condition | Severity | Why |
|---|---|---|---|
| **Reset-rate-limit storm** | Rate-limit triggers > 10 per 5-min window | Warning | Either a single attacker or a misbehaving FE retry loop. |
| **Enumeration probe** | Unknown-email rate > 20 per 5-min window | Warning | Someone is mapping the user base via `/request`. |
| **Email-send failure** | Failures > 5% of issuances over 15 min | Critical | MailerSend or config breakage — users can't get reset links. |
| **Anomalous completion drop** | Issuance→completion ratio drops <0.2 over 24h | Warning | Possible phishing campaign or systemic problem with the FE reset page. |
| **No activity at all** | Zero `/request` hits over 7 days | Informational | The endpoint may be unreachable (routing/DNS regression). Catches silent breakage. |

#### Optional future enhancement — Prometheus counters

If steady-state log volume hits a level where LogQL queries become expensive (rough threshold: tens of thousands of password-reset events per day), the right move is to add Prometheus counter instrumentation in [domain/src/password_reset.rs](../../domain/src/password_reset.rs). Counter names should mirror the log signals:

```
password_reset_endpoint_hit_total{endpoint="request|validate|complete"}
password_reset_link_issued_total
password_reset_completed_total
password_reset_rate_limit_total{kind="min_interval|daily_cap"}
password_reset_unknown_email_total
password_reset_email_send_failure_total
```

Both signals (log + counter) can coexist; the counter would just become the primary panel source. Not in v1 scope.

## Ops Maintenance

### Sweeping the `password_reset_attempts` audit table

**The sweep runs in-process, daily, with 30-day retention** — no external cron / ops setup required. At server startup, `init_server` in [`web/src/lib.rs`](../../web/src/lib.rs) spawns a `tokio` task that loops forever, calling `domain::password_reset::sweep_old_attempts(db, 30)` once every 24 hours.

This mirrors the existing session-deletion pattern: `tower_sessions::PostgresStore::continuously_delete_expired(60s)` is spawned in the same function for the `authorized_sessions` table. Putting password-reset attempt pruning on the same lifecycle keeps the operational model consistent — one mental model, one place to look in code, one log stream.

| Retention horizon | Status |
|---|---|
| 24 hours (**hard lower bound**) | The daily-cap rate-limit check looks back 24 hours; pruning younger rows would corrupt rate-limit state. `sweep_old_attempts` rejects `retention_days < 1` with a `Validation` error before any DELETE runs. |
| 30 days (**default, in-process**) | Configured in `web/src/lib.rs` as `RETENTION_DAYS = 30`. Long enough for security forensics (a user reports "someone kept trying to reset my password last week"); short enough to keep the table small. |

### Per-iteration behavior

Each daily wake-up:

1. Sleep 24 hours
2. Call `sweep_old_attempts(&db, 30)`
3. If `deleted > 0` → log at INFO with the count (`[password-reset-sweep] removed N attempt record(s) older than 30d`)
4. If `Err` → log at WARN, do **not** exit the loop (transient DB failures shouldn't take down the sweep for the lifetime of the process)
5. Repeat

A "missed sweep" alert: if no `[password-reset-sweep]` line appears in logs for >2 days, the spawn died or the process is stuck. Page on-call.

### Ad-hoc invocation (for incident response)

The Rust function and the equivalent SQL stay available for manual use — e.g. shrinking the table immediately after a load test, or running with a tighter retention temporarily.

```sql
-- Dry-run: how many rows would be removed?
SELECT COUNT(*) FROM refactor_platform.password_reset_attempts
WHERE attempted_at < NOW() - INTERVAL '30 days';

-- Actually delete:
DELETE FROM refactor_platform.password_reset_attempts
WHERE attempted_at < NOW() - INTERVAL '30 days';
```

Both are safe to run concurrently with live request traffic — PostgreSQL MVCC ensures an INSERT with `attempted_at = NOW()` is outside the `< cutoff` predicate, and the in-process daily sweep tolerates concurrent deletes.

### Why in-process rather than external cron

| Trade-off | In-process spawn (chosen) | External cron |
|---|---|---|
| Ops setup | None — automatic on every deploy | Requires cron config, separate auth/access to DB |
| Operational model consistency | Matches existing session-deletion task in the same file | Splits maintenance across two systems |
| Sweep runs after every deploy automatically | ✓ | Manual config to start |
| Multi-instance coordination | Each instance sweeps independently — DELETE is idempotent so duplication is fine at our scale | Single dedicated host avoids duplication |
| Failure visibility | App logs (same stream as everything else) | Separate cron log stream |

At single-instance scale (current deploy), in-process wins on every axis except multi-instance coordination — and we don't have multiple instances yet. Revisit if/when we scale horizontally; switching to external cron at that point is a 10-line change to `init_server`.

## Key Files

| File | Role |
|---|---|
| `migration/src/m20260513_000000_add_purpose_to_magic_link_tokens.rs` | Adds `purpose` column to `magic_link_tokens`, backfills existing rows to `setup` |
| `migration/src/m20260514_000000_add_password_reset_attempts.rs` | Creates the append-only `password_reset_attempts` audit table + composite index |
| `entity/src/token_purpose.rs` | `TokenPurpose` enum (Setup / PasswordReset) |
| `entity/src/magic_link_tokens.rs` | `purpose` field on the token model |
| `entity/src/password_reset_attempts.rs` | Audit-row entity |
| `entity_api/src/magic_link_token.rs` | Purpose-scoped `find_by_token_hash`, `delete_all_for_user`, `find_by_user_ids` |
| `entity_api/src/password_reset_attempt.rs` | `record`, `find_most_recent`, `count_since`, `delete_older_than` |
| `domain/src/password_reset.rs` | `request_password_reset()` (sync critical path), `process_reset_in_background()` (spawned), `validate_reset_token()`, `complete_password_reset()`, rate-limit check, `pad_handler_duration()`, `sweep_old_attempts()` ops function |
| `domain/src/emails.rs` | `send_password_reset_email()` following the [two-tier pattern](email_notifications.md#two-tier-pattern) |
| `web/src/controller/password_reset_controller.rs` | Three handlers (request / validate / complete) |
| `web/src/router.rs` | Route registration + `ApiDoc::paths(...)` registration + `PerIpThrottle` layer attached to the `/password-reset/*` route group |
| `web/src/middleware/throttle.rs` | `Throttle` trait, `ThrottlePolicy::AUTH_ENDPOINT`, `PerIpThrottle` impl — see [`throttling.md`](throttling.md) |
| `service/src/config.rs` | `password_reset_email_template_id`, `password_reset_email_url_path`, `password_reset_token_expiry_seconds` |

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `PASSWORD_RESET_EMAIL_TEMPLATE_ID` | *(none — required)* | MailerSend template ID. Template must accept personalization vars `first_name`, `last_name`, `password_reset_url`. |
| `PASSWORD_RESET_EMAIL_URL_PATH` | `/reset-password/{token}` | URL path template. The literal `{token}` placeholder is substituted with the raw token; the FE route is `/reset-password/[token]`. |
| `PASSWORD_RESET_TOKEN_EXPIRY_SECONDS` | `1800` (30 min) | Token lifetime. Shorter than the 24h `MAGIC_LINK_EXPIRY_SECONDS` because the user is actively at their keyboard when requesting reset. |

## Cross-References

- **Wire contract:** `PasswordResetEndpoints` v1 on the coordinator blackboard (single source of truth for request/response shapes; this doc cross-references but does not duplicate it).
- **Related architecture:** [email_notifications.md](email_notifications.md) (two-tier email pattern), [authentication_error_flow.md](authentication_error_flow.md) (login error propagation — same `password_auth` crate and error chain), [throttling.md](throttling.md) (per-IP rate-limit module shared across auth endpoints).
- **Frontend coordination:** `referrer_policy_token_pages` question on the coordinator blackboard (FE-side `Referrer-Policy` audit for `/setup/[token]` and `/reset-password/[token]` pages).
