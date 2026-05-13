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
