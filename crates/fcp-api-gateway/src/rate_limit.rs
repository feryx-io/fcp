//! Per-client rate limiting middleware using token bucket algorithm.

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use dashmap::DashMap;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;
use tracing::warn;

use crate::config::RateLimitConfig;

/// Per-client rate limiter state.
///
/// Each unique client IP gets its own token bucket, preventing one noisy
/// client from exhausting the quota for everyone else.
#[derive(Clone)]
pub struct RateLimiterState {
    /// Per-IP rate limiters
    per_ip: Arc<DashMap<IpAddr, Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>>,

    /// Configuration
    pub config: RateLimitConfig,
}

impl RateLimiterState {
    /// Create a new per-client rate limiter.
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            per_ip: Arc::new(DashMap::new()),
            config,
        }
    }

    /// Check if a request from the given IP is allowed.
    pub fn check_ip(&self, ip: IpAddr) -> bool {
        if !self.config.enabled {
            return true;
        }

        let limiter = self
            .per_ip
            .entry(ip)
            .or_insert_with(|| {
                let quota = Quota::per_minute(
                    NonZeroU32::new(self.config.default_requests_per_minute).unwrap(),
                )
                .allow_burst(NonZeroU32::new(self.config.burst_size).unwrap());
                Arc::new(RateLimiter::direct(quota))
            })
            .clone();

        limiter.check().is_ok()
    }
}

/// Rate limiting middleware — per-client IP.
pub async fn rate_limit_middleware(
    State(rate_limiter): State<RateLimiterState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Extract client IP from ConnectInfo, X-Forwarded-For, or X-Real-IP
    let client_ip = connect_info
        .map(|ci| ci.0.ip())
        .or_else(|| {
            req.headers()
                .get("X-Forwarded-For")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.trim().parse::<IpAddr>().ok())
        })
        .or_else(|| {
            req.headers()
                .get("X-Real-IP")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<IpAddr>().ok())
        })
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));

    if !rate_limiter.check_ip(client_ip) {
        warn!(
            client_ip = %client_ip,
            path = %req.uri().path(),
            method = %req.method(),
            "Rate limit exceeded"
        );
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(req).await)
}
