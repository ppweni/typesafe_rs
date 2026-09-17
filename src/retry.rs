use std::{
    collections::BTreeSet,
    time::{Duration, SystemTime},
};

use reqwest::header::HeaderMap;

use crate::{Error, Result};

/// Retry settings, copied into each call so concurrent calls have independent budgets.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Retries after the initial attempt. Set to zero to disable retries.
    pub max_retries: u32,
    pub backoff_initial: Duration,
    pub backoff_max: Duration,
    /// Fraction randomly subtracted from each backoff, in `0.0..=1.0`.
    pub backoff_jitter: f64,
    pub http_statuses: BTreeSet<u16>,
    pub respect_retry_after: bool,
    pub api_connection_error: bool,
    pub api_timeout_error: bool,
    /// Stops before a retry whose delay would reach this budget. In-flight attempts
    /// use the request timeout; this is not a hard deadline (matching the Python SDK).
    pub timeout: Option<Duration>,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            backoff_initial: Duration::from_millis(500),
            backoff_max: Duration::from_secs(5),
            backoff_jitter: 0.25,
            http_statuses: [408, 429].into_iter().chain(500..600).collect(),
            respect_retry_after: true,
            api_connection_error: true,
            api_timeout_error: true,
            timeout: Some(Duration::from_secs(30)),
        }
    }
}
impl RetryPolicy {
    pub fn disabled() -> Self {
        Self {
            max_retries: 0,
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.backoff_jitter) {
            return Err(Error::Configuration(
                "backoff_jitter must be between zero and one".into(),
            ));
        }
        if self.timeout.is_some_and(|timeout| timeout.is_zero()) {
            return Err(Error::Configuration(
                "retry timeout must be positive".into(),
            ));
        }
        if self.http_statuses.iter().any(|s| !(100..=599).contains(s)) {
            return Err(Error::Configuration(
                "retry HTTP statuses must be between 100 and 599".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn retryable(&self, error: &Error) -> bool {
        match error {
            Error::Timeout(_) => self.api_timeout_error,
            Error::Transport(error) => {
                self.api_connection_error
                    && (error.is_connect() || error.is_body() || error.is_request())
            }
            Error::Api(error) => self.http_statuses.contains(&error.status.as_u16()),
            _ => false,
        }
    }

    pub(crate) fn delay(&self, retry: u32, error: &Error) -> Duration {
        if self.respect_retry_after
            && let Error::Api(error) = error
            && let Some(delay) = error.retry_after()
        {
            return delay;
        }
        if self.backoff_initial.is_zero() || self.backoff_max.is_zero() {
            return Duration::ZERO;
        }
        // Clamp in floating point before converting to Duration, even for u32::MAX retries.
        let exponential = self.backoff_initial.as_secs_f64() * 2.0_f64.powf(retry as f64);
        let seconds = exponential.min(self.backoff_max.as_secs_f64())
            * (1.0 - fastrand::f64() * self.backoff_jitter);
        Duration::try_from_secs_f64(seconds).unwrap_or(self.backoff_max)
    }
}

pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    for (name, multiplier) in [("retry-after-ms", 0.001), ("retry-after", 1.0)] {
        let Some(raw) = headers.get(name).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        let raw = raw.trim();
        if let Ok(value) = raw.parse::<f64>() {
            if value.is_finite()
                && value >= 0.0
                && let Ok(delay) = Duration::try_from_secs_f64(value * multiplier)
            {
                return Some(delay);
            }
        } else if name == "retry-after"
            && let Ok(date) = httpdate::parse_http_date(raw)
        {
            return Some(date.duration_since(SystemTime::now()).unwrap_or_default());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_formats_and_precedence() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "1.5".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_millis(1500)));
        headers.insert("retry-after-ms", "25".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_millis(25)));
        headers.insert("retry-after-ms", "NaN".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_millis(1500)));
        headers.insert(
            "retry-after",
            "Wed, 21 Oct 2015 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers), Some(Duration::ZERO));
        for raw in ["NaN", "inf", "-1", "1e300", "garbage"] {
            headers.insert("retry-after", raw.parse().unwrap());
            assert_eq!(retry_after(&headers), None);
        }
    }

    #[test]
    fn backoff_is_bounded_even_at_extreme_attempt_counts() {
        let policy = RetryPolicy {
            backoff_jitter: 0.0,
            ..RetryPolicy::default()
        };
        let error = Error::InvalidInput(String::new());
        assert_eq!(policy.delay(0, &error), Duration::from_millis(500));
        assert_eq!(policy.delay(1, &error), Duration::from_secs(1));
        assert_eq!(policy.delay(u32::MAX, &error), Duration::from_secs(5));
    }
}
