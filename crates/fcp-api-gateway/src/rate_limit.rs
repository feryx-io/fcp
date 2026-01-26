//! Rate limiting middleware using token bucket algorithm.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use std::num::NonZeroU32;
use std::sync::Arc;
use tracing::warn;

use crate::config::RateLimitConfig;

/// Rate limiter state
#[derive(Clone)]
pub struct RateLimiterState {
    /// Global rate limiter (applies to all requests)
    limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,

    /// Configuration
    config: RateLimitConfig,
}

impl RateLimiterState {
    /// Create a new rate limiter
    pub fn new(config: RateLimitConfig) -> Self {
        // Create quota: requests_per_minute requests per 60 seconds with burst
        let quota = Quota::per_minute(
            NonZeroU32::new(config.default_requests_per_minute).unwrap()
        )
        .allow_burst(NonZeroU32::new(config.burst_size).unwrap());

        let limiter = Arc::new(RateLimiter::direct(quota));

        Self { limiter, config }
    }

    /// Check if request is allowed
    pub fn check(&self) -> bool {
        if !self.config.enabled {
            return true;
        }

        self.limiter.check().is_ok()
    }
}

/// Rate limiting middleware
pub async fn rate_limit_middleware(
    State(rate_limiter): State<RateLimiterState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Check rate limit
    if !rate_limiter.check() {
        warn!(
            path = %req.uri().path(),
            method = %req.method(),
            "Rate limit exceeded"
        );
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    // Continue to next middleware/handler
    Ok(next.run(req).await)
}
