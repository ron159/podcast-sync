use crate::{AppState, error::ApiError};
use axum::http::{HeaderMap, header};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub(crate) struct RateLimiter {
    maximum: u32,
    window: Arc<Mutex<RateWindow>>,
}

struct RateWindow {
    started_at: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn new(maximum: u32) -> Result<Self, &'static str> {
        if maximum == 0 {
            return Err("rate limit must be positive");
        }
        Ok(Self {
            maximum,
            window: Arc::new(Mutex::new(RateWindow {
                started_at: Instant::now(),
                count: 0,
            })),
        })
    }

    fn check(&self) -> Result<(), ApiError> {
        let mut window = self.window.lock().map_err(ApiError::internal)?;
        if window.started_at.elapsed() >= Duration::from_secs(60) {
            window.started_at = Instant::now();
            window.count = 0;
        }
        if window.count >= self.maximum {
            return Err(ApiError::RateLimited);
        }
        window.count += 1;
        Ok(())
    }
}

impl AppState {
    pub(crate) fn authorize(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        self.rate_limiter().check()?;
        let value = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;
        let (scheme, token) = value.split_once(' ').ok_or(ApiError::Unauthorized)?;
        if !scheme.eq_ignore_ascii_case("Bearer")
            || token.as_bytes().ct_eq(self.token()).unwrap_u8() != 1
        {
            return Err(ApiError::Unauthorized);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_rejects_requests_after_the_configured_count() {
        let limiter = RateLimiter::new(2).unwrap();
        assert!(limiter.check().is_ok());
        assert!(limiter.check().is_ok());
        assert!(matches!(limiter.check(), Err(ApiError::RateLimited)));
    }
}
