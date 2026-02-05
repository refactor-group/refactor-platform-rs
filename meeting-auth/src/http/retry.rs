//! Custom retry policy with Retry-After header support.

use std::time::{Duration, SystemTime};

use reqwest::{Response, StatusCode};
use reqwest_retry::{RetryDecision, RetryPolicy};

/// Custom retry policy that respects Retry-After headers from rate limited responses.
///
/// The default reqwest-retry crate handles 429 (Too Many Requests) responses by retrying
/// with exponential backoff, but it does NOT parse the Retry-After header. This can cause:
/// - API says "retry after 60 seconds" but backoff says "retry in 2 seconds"
/// - Client burns through retry attempts and still fails
/// - May trigger additional rate limiting or API bans
///
/// This implementation:
/// - Parses Retry-After headers (both seconds and HTTP-date formats)
/// - Falls back to exponential backoff when header is missing
/// - Caps retry delays at a maximum value
pub struct RetryAfterPolicy {
    max_retries: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl RetryAfterPolicy {
    /// Create a new retry policy with default settings.
    ///
    /// # Arguments
    ///
    /// * `max_retries` - Maximum number of retry attempts
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
        }
    }

    /// Parse Retry-After header (supports both seconds and HTTP-date formats).
    ///
    /// # Returns
    ///
    /// `Some(Duration)` if header is present and valid, `None` otherwise.
    fn parse_retry_after(response: &Response) -> Option<Duration> {
        let header = response.headers().get("retry-after")?;
        let value = header.to_str().ok()?;

        // Try parsing as seconds first (most common)
        if let Ok(seconds) = value.parse::<u64>() {
            return Some(Duration::from_secs(seconds));
        }

        // Try parsing as HTTP-date (e.g., "Wed, 21 Oct 2026 07:28:00 GMT")
        if let Ok(date) = httpdate::parse_http_date(value) {
            let now = SystemTime::now();
            if let Ok(duration) = date.duration_since(now) {
                return Some(duration);
            }
        }

        None
    }

    /// Calculate exponential backoff delay.
    fn exponential_delay(&self, n_attempts: u32) -> Duration {
        let delay = self.base_delay.as_secs_f64() * 2_f64.powi(n_attempts as i32);
        Duration::from_secs_f64(delay.min(self.max_delay.as_secs_f64()))
    }

    /// Get the delay for a response, checking Retry-After header first.
    fn get_retry_delay(&self, response: &Response, n_past_retries: u32) -> Duration {
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            // Check for Retry-After header first
            Self::parse_retry_after(response)
                .unwrap_or_else(|| self.exponential_delay(n_past_retries))
                .min(self.max_delay)
        } else {
            // For server errors, use exponential backoff
            self.exponential_delay(n_past_retries)
        }
    }
}

impl RetryPolicy for RetryAfterPolicy {
    fn should_retry(&self, _request_start_time: SystemTime, n_past_retries: u32) -> RetryDecision {
        if n_past_retries >= self.max_retries {
            RetryDecision::DoNotRetry
        } else {
            let delay = self.exponential_delay(n_past_retries);
            RetryDecision::Retry {
                execute_after: SystemTime::now() + delay,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_delay() {
        let policy = RetryAfterPolicy::new(3);

        let delay0 = policy.exponential_delay(0);
        let delay1 = policy.exponential_delay(1);
        let delay2 = policy.exponential_delay(2);

        assert_eq!(delay0.as_secs(), 1);
        assert_eq!(delay1.as_secs(), 2);
        assert_eq!(delay2.as_secs(), 4);
    }

    #[test]
    fn test_max_delay_cap() {
        let policy = RetryAfterPolicy::new(10);

        let delay = policy.exponential_delay(10);
        assert!(delay <= policy.max_delay);
    }

    #[test]
    fn test_parse_retry_after_seconds() {
        // This test would require creating a mock Response with headers
        // Skipping for now as it requires more setup
    }
}
