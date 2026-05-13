# Request Throttling Architecture

Per-IP rate limiting applied to abuse-prone endpoint surfaces. Lives in [`web::middleware::throttle`](../../web/src/middleware/throttle.rs) and is opt-in per route group.

## Why per-IP throttling exists alongside per-resource rate limits

Per-resource rate limits (e.g. the per-email limit in [password_reset.md](password_reset.md)) and per-IP rate limits defend **disjoint** attack classes:

| Limit type | Defends | Doesn't defend |
|---|---|---|
| Per-resource (per-email, per-user) | Targeted abuse against one specific resource — e.g. inbox flooding for one user | Mass scanning that varies the resource per request |
| Per-IP | Mass scanning (one attacker varying emails/tokens per request) | Distributed botnet abuse (each IP looks legitimate) |

For a public unauthenticated endpoint like password-reset, both are required. Per-email defeats targeted Alice-flooding; per-IP defeats Mallory scanning a million addresses to enumerate the user base or burn email-provider credit.

A common mistake — which a PR #311 review caught — is to treat the two as comparable defenses ("we have one rate limit"). They aren't. If you can list your rate-limit keys and an attacker can change all of them per request at line rate, you have no effective rate limit on that endpoint.

## Module structure

```
web/src/middleware/throttle.rs
├── trait Throttle              ← interface
├── struct ThrottlePolicy       ← named const policies (e.g. AUTH_ENDPOINT)
└── struct PerIpThrottle        ← in-process tower_governor implementation
```

### `trait Throttle`

```rust
pub trait Throttle {
    type Layer;
    fn into_layer(self) -> Self::Layer;
}
```

Single method: build a `tower::Layer` to attach to a `Router`. The associated `Layer` type lets each implementation expose its own concrete layer (e.g. `GovernorLayer` for the in-process impl, a hypothetical `RedisGovernorLayer` for a future shared-state impl) without erasing the type or paying for dynamic dispatch.

The trait exists so future implementations can be swapped in without changing route definitions. Today there is one production implementation; the abstraction defines the swap point.

### `struct ThrottlePolicy`

Named throttle configurations — what counts as "too many requests."

```rust
pub struct ThrottlePolicy {
    pub period_secs: u64,   // seconds between token replenishments
    pub burst: u32,         // initial token capacity / burst allowance
}

impl ThrottlePolicy {
    pub const AUTH_ENDPOINT: Self = Self { period_secs: 6, burst: 10 };
}
```

| Policy | Sustained rate | Burst | Intended for |
|---|---|---|---|
| `AUTH_ENDPOINT` | ~10 req/min per IP | 10 | Unauthenticated credential-recovery / login flows: password-reset, magic-link, future signup/login |

**When to add a new policy**: only when an endpoint genuinely needs a different rate. If you're tempted to add `LENIENT_AUTH_ENDPOINT` because the existing policy is "too strict," that's usually a sign the existing policy is mis-calibrated, not that a second one is needed. Discuss the tradeoff first; converging on a small set of named policies prevents per-route config sprawl.

### `struct PerIpThrottle`

The production implementation. Wraps `tower_governor` with `SmartIpKeyExtractor` and an in-process token bucket.

```rust
pub struct PerIpThrottle { policy: ThrottlePolicy }

impl PerIpThrottle {
    pub fn new(policy: ThrottlePolicy) -> Self { ... }
}

impl Throttle for PerIpThrottle {
    type Layer = GovernorLayer<SmartIpKeyExtractor, NoOpMiddleware<QuantaInstant>>;
    fn into_layer(self) -> Self::Layer { ... }
}
```

Each call to `into_layer()` builds a fresh in-process governor — state is per-instance, not shared across processes. Buckets are keyed by client IP extracted via `SmartIpKeyExtractor`.

## Trust assumption (critical)

`SmartIpKeyExtractor` resolves the client IP from headers in this priority:

1. `Forwarded` header (RFC 7239 standardized)
2. `X-Forwarded-For` header (de facto standard, set by most reverse proxies)
3. Peer socket address (fallback — what `tcp.accept()` returned)

**Behind a trusted reverse proxy** (nginx in our deploys) the headers are set by infrastructure we control, so they accurately identify the client. The throttle works per-client.

**Without a reverse proxy** (or with one that doesn't strip client-provided `X-Forwarded-For`), the headers are spoofable. An attacker who controls the headers can present a different IP per request and bypass the throttle entirely. **This is the failure mode to watch for** when deploying:

- ✅ Production / staging / PR-preview environments behind project nginx → safe
- ✅ Local dev (no proxy) → falls back to peer address `127.0.0.1`; throttle becomes effectively process-wide, which is acceptable for dev (not a security regression — the threat model doesn't apply locally)
- ❌ Production-style deploy without nginx, or with a misconfigured nginx that passes through client-provided `X-Forwarded-For` → silently broken; treat as a security incident

Future maintainers: if you remove nginx or change the proxy layout, the `Throttle` interface's documentation contract is broken. Read the trust assumption in [`throttle.rs`](../../web/src/middleware/throttle.rs) and update both code and docs together.

## Current applications

| Endpoint group | Policy | Layer attached |
|---|---|---|
| `/password-reset/*` | `AUTH_ENDPOINT` | [`web::router::password_reset_routes`](../../web/src/router.rs) |

Future candidates (not yet throttled — add when they ship):

- `/magic-link/*` — same threat profile as password-reset (unauthenticated, email-issuing). The reason it's not throttled today is grandfathered; a follow-up should add `AUTH_ENDPOINT` throttling.
- `/login` — once we implement signup or login lockout / brute-force protection.

## Adding throttling to a new endpoint

1. Pick the right `ThrottlePolicy` const. If existing policies don't fit, propose a new const with rationale rather than building inline.
2. In the route definition, attach the layer:

```rust
use crate::middleware::throttle::{PerIpThrottle, Throttle, ThrottlePolicy};

fn my_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/my-endpoint", post(handler))
        .layer(PerIpThrottle::new(ThrottlePolicy::AUTH_ENDPOINT).into_layer())
        .with_state(app_state)
}
```

3. Update this doc's "Current applications" table.
4. Document the new threat-model entries in the feature's own arch doc.

## Out-of-scope / known limitations

### Horizontal scaling

In-process state means each instance keeps its own per-IP buckets. With `N` instances behind a load balancer, the effective rate limit is `N × policy.requests`. Acceptable for the current single-instance deploy; not acceptable past 2-3 instances.

The fix when scaling becomes a concern: introduce a `RedisBackedThrottle` implementing the same `Throttle` trait. Call sites under `impl Throttle` don't change. Open work, not in v1.

### Distributed attacks

Per-IP throttling is defeated by attackers controlling many IPs (botnet, residential proxy pool, IPv6 prefix abuse). Defensive layers below per-IP would include:

- Per-AS throttling (group IPs by network owner)
- CAPTCHA challenges on suspicious traffic
- Reputation-based throttling (block known abuse sources)

None of these are in v1 — they belong at the edge (CDN / WAF) rather than the application layer. If we ever sit behind Cloudflare, configure it there.

### 429 response shape

`tower_governor`'s default 429 response is plain-text `"Too Many Requests"` plus a `Retry-After` header. We accept this for the throttle layer rather than customizing — it sits at the outer ring of defense and the FE handles 429 generically. Note that the *inner* per-resource rate limits (e.g. password-reset's per-email 429) use the project's structured `{ status_code, error, message }` JSON shape via the error mapper in [`web::error`](../../web/src/error.rs). The two 429 shapes coexist.

## Cross-References

- Implementation: [`web::middleware::throttle`](../../web/src/middleware/throttle.rs)
- Underlying crate: [`tower_governor`](https://crates.io/crates/tower_governor) (which wraps [`governor`](https://crates.io/crates/governor))
- Per-resource rate limits: [`password_reset.md`](password_reset.md) for the per-email rate limit on `/password-reset/request`
- Key extraction trust model: documented in [`throttle.rs`](../../web/src/middleware/throttle.rs)
