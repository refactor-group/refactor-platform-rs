//! Simple exponential backoff retry policy.

use std::time::{Duration, SystemTime};

use reqwest_retry::{RetryDecision, RetryPolicy};

/// Exponential backoff retry policy.
///
/// Retries failed requests with exponentially increasing delays, capped at a maximum.
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

    /// Calculate exponential backoff delay.
    fn exponential_delay(&self, n_attempts: u32) -> Duration {
        let delay = self.base_delay.as_secs_f64() * 2_f64.powi(n_attempts as i32);
        Duration::from_secs_f64(delay.min(self.max_delay.as_secs_f64()))
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
}
