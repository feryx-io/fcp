//! Caching layer using Redis for GET requests.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use redis::AsyncCommands;
use tracing::{debug, error, warn};

use crate::config::{CacheConfig, RedisConfig};

/// Cache state
#[derive(Clone)]
pub struct CacheState {
    /// Redis connection manager
    client: Option<redis::Client>,

    /// Configuration
    config: CacheConfig,
}

impl CacheState {
    /// Create a new cache state
    pub fn new(redis_config: &RedisConfig, cache_config: CacheConfig) -> Self {
        let client = if cache_config.enabled {
            match redis::Client::open(redis_config.url.as_str()) {
                Ok(client) => {
                    debug!("Redis client created successfully");
                    Some(client)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to create Redis client, caching disabled");
                    None
                }
            }
        } else {
            None
        };

        Self {
            client,
            config: cache_config,
        }
    }

    /// Get cached response from Redis
    async fn get_cached(&self, key: &str) -> Option<Vec<u8>> {
        let client = self.client.as_ref()?;

        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => match conn.get::<_, Vec<u8>>(key).await {
                Ok(data) => {
                    debug!(key = %key, size = data.len(), "Cache hit");
                    Some(data)
                }
                Err(_) => None,
            },
            Err(e) => {
                error!(error = %e, "Failed to get Redis connection");
                None
            }
        }
    }

    /// Store response in Redis
    async fn set_cached(&self, key: &str, value: Vec<u8>) {
        let client = match self.client.as_ref() {
            Some(c) => c,
            None => return,
        };

        let ttl_secs = self.config.default_ttl.as_secs();

        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                if let Err(e) = conn.set_ex::<_, _, ()>(key, value, ttl_secs).await {
                    error!(error = %e, "Failed to cache response");
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to get Redis connection for caching");
            }
        }
    }

    /// Generate cache key from request, incorporating auth context to prevent cross-user leakage.
    fn cache_key(&self, req: &Request<Body>) -> String {
        // Include Authorization header hash in cache key for tenant/user isolation
        let auth_hash = req
            .headers()
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .map(|auth| {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                auth.hash(&mut hasher);
                format!(":{:x}", hasher.finish())
            })
            .unwrap_or_default();

        // Include X-Tenant-Id if present
        let tenant = req
            .headers()
            .get("X-Tenant-Id")
            .and_then(|v| v.to_str().ok())
            .map(|t| format!(":t={}", t))
            .unwrap_or_default();

        format!(
            "gateway:cache:{}{}{}{}",
            req.uri().path(),
            req.uri()
                .query()
                .map(|q| format!("?{}", q))
                .unwrap_or_default(),
            auth_hash,
            tenant,
        )
    }
}

/// Paths that must NEVER be cached (user-specific or sensitive).
const UNCACHEABLE_PREFIXES: &[&str] = &[
    "/api/v1/auth/",
    "/api/v1/me/",
    "/api/v1/events",
];

/// Cache middleware
pub async fn cache_middleware(
    State(cache): State<CacheState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Skip caching if disabled, or if get_only mode and request is not GET
    if !cache.config.enabled || (cache.config.get_only && req.method() != "GET") {
        return Ok(next.run(req).await);
    }

    // Never cache auth/user-specific endpoints
    let path = req.uri().path();
    if UNCACHEABLE_PREFIXES.iter().any(|prefix| path.starts_with(prefix)) {
        return Ok(next.run(req).await);
    }

    // Generate cache key (includes auth + tenant context)
    let cache_key = cache.cache_key(&req);

    // Check cache
    if let Some(cached_data) = cache.get_cached(&cache_key).await {
        debug!(key = %cache_key, "Serving from cache");
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("X-Cache", "HIT")
            .header("Content-Type", "application/json")
            .body(Body::from(cached_data))
            .unwrap());
    }

    // Cache miss - forward to backend
    let response = next.run(req).await;

    // Cache successful GET responses
    if response.status().is_success() {
        // Extract body bytes for caching
        let (parts, body) = response.into_parts();
        match axum::body::to_bytes(body, usize::MAX).await {
            Ok(bytes) => {
                // Cache the response
                let cache_clone = cache.clone();
                let key_clone = cache_key.clone();
                let bytes_clone = bytes.to_vec();
                tokio::spawn(async move {
                    cache_clone.set_cached(&key_clone, bytes_clone).await;
                });

                // Return response with cache miss header
                let mut response = Response::from_parts(parts, Body::from(bytes));
                response
                    .headers_mut()
                    .insert("X-Cache", "MISS".parse().unwrap());
                Ok(response)
            }
            Err(e) => {
                error!(error = %e, "Failed to read response body for caching");
                // Return error response
                Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("Failed to process response"))
                    .unwrap())
            }
        }
    } else {
        // Don't cache non-success responses
        Ok(response)
    }
}
