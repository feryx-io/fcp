//! Request limits for API tokens (docs/DEVELOPER_SURFACE.md §3, phase 2).
//!
//! Each API token, and all of a tenant's tokens together, may make so many
//! requests a minute (`[api.rate_limit]`). Counts are kept in fixed
//! one-minute windows by a [`RateCounter`]: the admin's Redis when it has
//! one, so every admin node counts together, else this process. A request
//! past either limit is answered 429 with `Retry-After`; every limited
//! request carries `RateLimit-Limit`, `RateLimit-Remaining` and
//! `RateLimit-Reset` for the tighter of the two. Sessions and the static
//! token are not limited. A counter that fails lets the request through: a
//! broken Redis should not take the API down with it.

use crate::auth::AuthIdentity;
use crate::error::ErrorResponse;
use crate::state::ApiState;
use async_trait::async_trait;
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use dashmap::DashMap;
use fcp_core::ApiRateLimitConfig;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const LIMIT_HEADER: &str = "ratelimit-limit";
pub const REMAINING_HEADER: &str = "ratelimit-remaining";
pub const RESET_HEADER: &str = "ratelimit-reset";

/// The length of a counting window.
pub const WINDOW: Duration = Duration::from_secs(60);

/// Counts requests against a key in fixed windows.
#[async_trait]
pub trait RateCounter: Send + Sync {
    /// Count one request against `key` in the window numbered `window`
    /// (seconds since the epoch / the window's length), returning the count
    /// so far including this one. The count may be forgotten once the window
    /// has passed.
    async fn hit(&self, key: &str, window: u64) -> anyhow::Result<u64>;
}

/// Counts in this process: right for one admin node.
#[derive(Default)]
pub struct LocalCounter {
    counts: DashMap<String, (u64, u64)>,
}

#[async_trait]
impl RateCounter for LocalCounter {
    async fn hit(&self, key: &str, window: u64) -> anyhow::Result<u64> {
        let mut entry = self.counts.entry(key.to_string()).or_insert((window, 0));
        if entry.0 != window {
            *entry = (window, 0);
        }
        entry.1 += 1;
        let count = entry.1;
        drop(entry);
        // Forget keys whose window has passed, now and then.
        if count == 1 && self.counts.len() > 10_000 {
            self.counts.retain(|_, (w, _)| *w == window);
        }
        Ok(count)
    }
}

/// The limits and where they are counted.
pub struct RateLimiter {
    config: ApiRateLimitConfig,
    counter: Arc<dyn RateCounter>,
}

impl RateLimiter {
    pub fn new(config: ApiRateLimitConfig, counter: Arc<dyn RateCounter>) -> Self {
        Self { config, counter }
    }

    /// The limits that apply to `identity`, as `(key, limit)`: a token's own
    /// and its tenant's. None for a session or the static token.
    fn keys(&self, identity: &AuthIdentity) -> Vec<(String, u32)> {
        let (token_id, tenant_id) = match identity {
            AuthIdentity::ApiToken {
                token_id,
                tenant_id,
                ..
            }
            | AuthIdentity::Personal {
                token_id,
                tenant_id,
                ..
            } => (token_id, tenant_id),
            _ => return Vec::new(),
        };
        vec![
            (
                format!("token:{token_id}"),
                self.config.per_token_per_minute,
            ),
            (
                format!("tenant:{tenant_id}"),
                self.config.per_tenant_per_minute,
            ),
        ]
    }
}

/// What a request's counts came to: the tightest limit's numbers.
struct Standing {
    limit: u32,
    remaining: u32,
    reset_secs: u64,
    exceeded: bool,
}

async fn stand(limiter: &RateLimiter, identity: &AuthIdentity) -> Option<Standing> {
    if !limiter.config.enabled {
        return None;
    }
    let keys = limiter.keys(identity);
    if keys.is_empty() {
        return None;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let window = now / WINDOW.as_secs();
    let reset_secs = (window + 1) * WINDOW.as_secs() - now;
    let mut tightest: Option<Standing> = None;
    for (key, limit) in keys {
        let count = match limiter.counter.hit(&key, window).await {
            Ok(count) => count,
            Err(e) => {
                tracing::warn!(error = %e, key, "Rate counter failed; request let through");
                continue;
            }
        };
        let remaining = u64::from(limit).saturating_sub(count) as u32;
        let standing = Standing {
            limit,
            remaining,
            reset_secs,
            exceeded: count > u64::from(limit),
        };
        let tighter = match &tightest {
            None => true,
            Some(t) => {
                (standing.exceeded && !t.exceeded)
                    || (standing.exceeded == t.exceeded && standing.remaining < t.remaining)
            }
        };
        if tighter {
            tightest = Some(standing);
        }
    }
    tightest
}

fn number(n: impl ToString) -> HeaderValue {
    HeaderValue::from_str(&n.to_string()).expect("a number is a header value")
}

/// Count the request against its token's and tenant's limits.
pub async fn enforce(State(state): State<ApiState>, req: Request, next: Next) -> Response {
    let Some(identity) = req.extensions().get::<AuthIdentity>().cloned() else {
        return next.run(req).await;
    };
    let Some(standing) = stand(&state.rate_limiter, &identity).await else {
        return next.run(req).await;
    };
    let mut response = if standing.exceeded {
        let mut refused = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse::for_status(
                StatusCode::TOO_MANY_REQUESTS,
                Some(format!(
                    "More than {} requests this minute; retry in {} s",
                    standing.limit, standing.reset_secs
                )),
            )),
        )
            .into_response();
        refused
            .headers_mut()
            .insert(header::RETRY_AFTER, number(standing.reset_secs));
        refused
    } else {
        next.run(req).await
    };
    let headers = response.headers_mut();
    headers.insert(LIMIT_HEADER, number(standing.limit));
    headers.insert(REMAINING_HEADER, number(standing.remaining));
    headers.insert(RESET_HEADER, number(standing.reset_secs));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: &str, tenant: fcp_core::TenantId) -> AuthIdentity {
        AuthIdentity::ApiToken {
            token_id: id.to_string(),
            tenant_id: tenant,
            permissions: vec![],
        }
    }

    #[tokio::test]
    async fn a_token_past_its_limit_is_refused_until_the_window_turns() {
        let counter = Arc::new(LocalCounter::default());
        assert_eq!(counter.hit("k", 7).await.unwrap(), 1);
        assert_eq!(counter.hit("k", 7).await.unwrap(), 2);
        assert_eq!(
            counter.hit("k", 8).await.unwrap(),
            1,
            "a new window starts at one"
        );

        let limiter = RateLimiter::new(
            ApiRateLimitConfig {
                enabled: true,
                per_token_per_minute: 2,
                per_tenant_per_minute: 100,
            },
            counter,
        );
        let tenant = fcp_core::TenantId::new();
        let a = token("a", tenant);
        let first = stand(&limiter, &a).await.unwrap();
        assert_eq!(
            (first.limit, first.remaining, first.exceeded),
            (2, 1, false)
        );
        assert!(!stand(&limiter, &a).await.unwrap().exceeded);
        let third = stand(&limiter, &a).await.unwrap();
        assert!(third.exceeded);
        assert!(third.reset_secs <= 60);
        // Another token of the tenant has its own count.
        assert!(!stand(&limiter, &token("b", tenant)).await.unwrap().exceeded);
    }

    #[tokio::test]
    async fn a_tenant_is_limited_across_its_tokens() {
        let limiter = RateLimiter::new(
            ApiRateLimitConfig {
                enabled: true,
                per_token_per_minute: 100,
                per_tenant_per_minute: 3,
            },
            Arc::new(LocalCounter::default()),
        );
        let tenant = fcp_core::TenantId::new();
        for id in ["a", "b", "c"] {
            assert!(!stand(&limiter, &token(id, tenant)).await.unwrap().exceeded);
        }
        let fourth = stand(&limiter, &token("d", tenant)).await.unwrap();
        assert!(fourth.exceeded);
        assert_eq!(fourth.limit, 3);
        // Another tenant is untouched.
        let other = fcp_core::TenantId::new();
        assert!(!stand(&limiter, &token("e", other)).await.unwrap().exceeded);
    }

    #[tokio::test]
    async fn sessions_and_the_static_token_are_not_limited() {
        let limiter = RateLimiter::new(
            ApiRateLimitConfig {
                enabled: true,
                per_token_per_minute: 0,
                per_tenant_per_minute: 0,
            },
            Arc::new(LocalCounter::default()),
        );
        assert!(stand(&limiter, &AuthIdentity::StaticToken).await.is_none());
        let session = AuthIdentity::Session {
            username: "u".into(),
            role: "tenant_admin".into(),
            tenant_id: None,
            user_id: None,
            session_id: None,
        };
        assert!(stand(&limiter, &session).await.is_none());
    }
}
