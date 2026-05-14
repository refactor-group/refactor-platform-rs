//! Per-IP request throttling middleware.
//!
//! Exposes a small trait, `Throttle`, with a single production
//! implementation: `PerIpThrottle` backed by `tower_governor`'s in-process
//! token bucket. The trait exists so future implementations (e.g. a
//! `RedisBackedThrottle` for sharing state across horizontally-scaled
//! instances) can be swapped in without changing route definitions —
//! call sites take `impl Throttle` and depend only on `into_layer()`.
//!
//! Typical call site:
//!
//! ```ignore
//! Router::new()
//!     .route("/some-auth-flow", post(handler))
//!     .layer(PerIpThrottle::new(ThrottlePolicy::AUTH_ENDPOINT).into_layer())
//! ```
//!
//! See `docs/architecture/throttling.md` for the design, trust assumption,
//! and horizontal-scaling notes.

use governor::middleware::NoOpMiddleware;
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};

/// A request throttle that can produce a tower layer to attach to a Router.
///
/// Implementations differ in *where* the rate-limit state lives (process,
/// Redis, etc.) and *what* key the limit applies to (per-IP, per-user,
/// per-organization, etc.). The trait isolates those choices from the
/// route definition.
pub trait Throttle {
    /// The concrete `tower::Layer` type this throttle produces. Each
    /// implementation exposes its own layer type (e.g. `GovernorLayer`
    /// for the in-process governor); the trait abstracts over the choice
    /// at the call site.
    type Layer;

    /// Build the layer, consuming the throttle config. Call inside route
    /// setup and attach via `.layer(...)`.
    fn into_layer(self) -> Self::Layer;
}

/// A named throttle configuration — how strict the rate limit is.
///
/// Construct via the associated `const`s rather than building inline,
/// so the rate-limit arithmetic stays in one place and call sites read
/// `ThrottlePolicy::AUTH_ENDPOINT` instead of `period_secs: 6, burst: 10`.
#[derive(Debug, Clone, Copy)]
pub struct ThrottlePolicy {
    /// Seconds between token replenishments.
    pub period_secs: u64,
    /// Initial token capacity / burst allowance.
    pub burst: u32,
}

impl ThrottlePolicy {
    /// Strict policy for authentication / credential-recovery endpoints
    /// — password reset, magic-link, future login / signup. Approximately
    /// **10 req/min sustained per IP with a burst of 10**.
    ///
    /// Designed to defeat mass-scanning attacks (an attacker varying
    /// emails or tokens per request cannot exceed line-rate this slow)
    /// while leaving room for legitimate user retries.
    ///
    /// Use for any new endpoint that: (1) is unauthenticated, AND
    /// (2) either issues an email, mutates user credentials, or returns
    /// information whose value scales with the number of requests
    /// (e.g. enumeration probes).
    pub const AUTH_ENDPOINT: Self = Self {
        period_secs: 6,
        burst: 10,
    };
}

/// Per-IP throttle backed by an in-process token bucket (`tower_governor`).
///
/// Key extraction uses [`SmartIpKeyExtractor`], which trusts
/// `Forwarded` / `X-Forwarded-For` headers. This is correct behind a
/// trusted reverse proxy (nginx); without one the headers are spoofable.
/// See `docs/architecture/throttling.md` for the trust model.
///
/// State is per-process (in-memory). When the backend scales horizontally,
/// each instance keeps its own buckets per IP — the effective limit becomes
/// `N × policy.requests` across `N` instances. Documented as future work
/// in the arch doc; a `RedisBackedThrottle` would share state across the
/// fleet.
pub struct PerIpThrottle {
    policy: ThrottlePolicy,
}

impl PerIpThrottle {
    pub fn new(policy: ThrottlePolicy) -> Self {
        Self { policy }
    }
}

// Compile-time policy bounds check. If someone "tunes" AUTH_ENDPOINT
// to looser values without thinking about the threat model, the BUILD
// fails — strictly stronger than a runtime test, which only fires if
// tests are actually run. The error messages show up at compile time.
const _AUTH_ENDPOINT_BOUNDS_CHECK: () = {
    assert!(
        ThrottlePolicy::AUTH_ENDPOINT.period_secs >= 6,
        "AUTH_ENDPOINT must replenish no faster than 10/min (period_secs >= 6) — \
         loosening this is a policy change that needs threat-model review"
    );
    assert!(
        ThrottlePolicy::AUTH_ENDPOINT.burst <= 20,
        "AUTH_ENDPOINT burst > 20 lets attackers spray a large initial volume \
         before throttling kicks in"
    );
};

impl Throttle for PerIpThrottle {
    type Layer = GovernorLayer<SmartIpKeyExtractor, NoOpMiddleware<governor::clock::QuantaInstant>>;

    fn into_layer(self) -> Self::Layer {
        let config = Arc::new(
            GovernorConfigBuilder::default()
                .per_second(self.policy.period_secs)
                .burst_size(self.policy.burst)
                .key_extractor(SmartIpKeyExtractor)
                .finish()
                .expect("invalid throttle config (period_secs and burst must be > 0)"),
        );
        GovernorLayer { config }
    }
}

#[cfg(test)]
mod tests {
    //! Mocked-load tests for the per-IP throttle.
    //!
    //! These test our *wiring* of `tower_governor` (key-extractor choice,
    //! burst size, layer attachment) rather than the library's internals.
    //! Each test fires a rapid sequence of requests against a minimal
    //! Router with the throttle layer attached and asserts the
    //! 200/429 split matches the configured policy.
    //!
    //! All tests use `X-Forwarded-For` to simulate per-client IPs because
    //! `SmartIpKeyExtractor` prefers that header. `oneshot` doesn't carry
    //! a real peer address, so without the header the extractor would
    //! fail or fall back unpredictably.

    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    /// Test handler that always returns 200 OK. Lets us isolate throttle
    /// behavior from any business-logic response codes.
    async fn ok_handler() -> impl IntoResponse {
        (StatusCode::OK, "ok")
    }

    /// Build a Router with the AUTH_ENDPOINT throttle attached to a
    /// trivial /test handler. Each call to this function produces a Router
    /// with its own underlying rate limiter (state is per-layer).
    fn test_app() -> Router {
        Router::new()
            .route("/test", get(ok_handler))
            .layer(PerIpThrottle::new(ThrottlePolicy::AUTH_ENDPOINT).into_layer())
    }

    /// Build a request with a specific simulated client IP via
    /// `X-Forwarded-For` (which `SmartIpKeyExtractor` picks up).
    fn req_from_ip(ip: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri("/test")
            .header("X-Forwarded-For", ip)
            .body(Body::empty())
            .unwrap()
    }

    /// Burst behavior: the first `burst` requests from one IP must succeed
    /// (200), and excess requests in the same window must be throttled
    /// (429). This is the load-bearing "rate limit actually fires" test.
    #[tokio::test]
    async fn per_ip_throttle_allows_burst_then_429s() {
        let app = test_app();
        let burst = ThrottlePolicy::AUTH_ENDPOINT.burst as usize;
        let total = burst + 3;

        let mut statuses = Vec::with_capacity(total);
        for _ in 0..total {
            let res = app
                .clone()
                .oneshot(req_from_ip("203.0.113.1"))
                .await
                .unwrap();
            statuses.push(res.status());
        }

        let ok_count = statuses.iter().filter(|s| s.is_success()).count();
        let throttled_count = statuses
            .iter()
            .filter(|s| **s == StatusCode::TOO_MANY_REQUESTS)
            .count();

        assert_eq!(
            ok_count, burst,
            "expected exactly {burst} OK responses (one full burst); got statuses: {statuses:?}"
        );
        assert_eq!(
            throttled_count,
            total - burst,
            "expected {} throttled responses; got statuses: {statuses:?}",
            total - burst
        );
        // Sanity: the OKs come first, the 429s come after.
        for (i, status) in statuses.iter().enumerate() {
            if i < burst {
                assert_eq!(*status, StatusCode::OK, "request {i} should be 200");
            } else {
                assert_eq!(
                    *status,
                    StatusCode::TOO_MANY_REQUESTS,
                    "request {i} should be 429"
                );
            }
        }
    }

    /// Per-IP isolation: exhausting IP-A's bucket must NOT affect IP-B.
    /// This proves the `SmartIpKeyExtractor` is actually keying per-IP
    /// (not globally collapsing to a single bucket). A misconfiguration
    /// where, say, the wrong extractor returned a constant key would
    /// fail this test.
    #[tokio::test]
    async fn per_ip_throttle_isolates_per_ip() {
        let app = test_app();
        let burst = ThrottlePolicy::AUTH_ENDPOINT.burst as usize;

        // Exhaust IP-A's bucket.
        for i in 0..burst {
            let res = app
                .clone()
                .oneshot(req_from_ip("203.0.113.10"))
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::OK,
                "IP-A request {i} should be 200"
            );
        }

        // IP-A should now be throttled.
        let res = app
            .clone()
            .oneshot(req_from_ip("203.0.113.10"))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "IP-A should be throttled after burst exhausted"
        );

        // IP-B must have its own untouched bucket.
        let res = app
            .clone()
            .oneshot(req_from_ip("203.0.113.20"))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "IP-B must have its own bucket — exhausting IP-A must not throttle IP-B"
        );
    }

    // Sanity check on AUTH_ENDPOINT policy bounds is now a compile-time
    // const assertion above (see `_AUTH_ENDPOINT_BOUNDS_CHECK`). Stronger
    // than a runtime test — the build fails if someone loosens the policy.
}
