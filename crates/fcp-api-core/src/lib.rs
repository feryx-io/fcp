//! FCP REST API core functionality.
//!
//! This crate provides the REST API implementation used by FCP services.
//! It includes:
//!
//! - Authentication and authorization
//! - API error handling
//! - State management
//! - v1 API endpoints
//! - Request validation
//! - Router creation and server startup

pub mod auth;
pub mod error;
pub mod state;
pub mod trunk_health;
pub mod v1;
pub mod validation;

// Re-export commonly used types
pub use error::ApiError;
pub use state::{ApiState, SecurityPolicy};

use axum::{
    http::{header, Method},
    middleware, Router,
};
use fcp_core::ApiConfig;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

/// Create the API router with all routes and middleware.
///
/// # Panics
///
/// Panics if `auth_enabled` is true but `auth_token` is not configured.
/// This is intentional — fail closed rather than silently running without auth.
pub fn create_router(mut state: ApiState, api_config: &ApiConfig) -> Router {
    // CORS configuration
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    // Build the auth-protected API router
    let mut router = Router::new()
        .route(
            "/metrics",
            axum::routing::get(v1::metrics::prometheus_metrics)
                .route_layer(middleware::from_fn(auth::require_admin)),
        )
        .nest("/api/v1", v1::router());

    // Always derive JWT secret from auth_token when available.
    // JWT session auth (admin UI login) is independent of bearer middleware.
    if let Some(ref auth_token) = api_config.auth_token {
        state = state.with_jwt_secret_from_token(auth_token);
    }
    state = state.with_provisioning_auth_token(api_config.auth_token.clone());

    // Conditionally apply bearer token authentication middleware
    if api_config.auth_enabled {
        let auth_token = api_config.auth_token.as_ref().unwrap_or_else(|| {
            panic!(
                "FATAL: auth_enabled=true but no auth_token configured. \
                 Set api.auth_token in config or disable auth with api.auth_enabled=false."
            )
        });

        tracing::info!("API authentication enabled");
        let mut auth_config = auth::AuthConfig::new(auth_token.clone());
        // Share JWT secret so bearer middleware also accepts JWT session tokens
        if let Some(ref jwt_secret) = state.jwt_secret {
            auth_config = auth_config.with_jwt_secret(jwt_secret.clone());
        }
        // Share credential store so JWT auth can verify user is still active
        if let Some(ref storage) = state.storage {
            auth_config = auth_config.with_credential_store(storage.credential_store_arc());
        }
        // Store auth config in ApiState for WebSocket first-message auth
        state.auth_config = Some(auth_config.clone());
        router = router.layer(middleware::from_fn(move |req, next| {
            let config = auth_config.clone();
            auth::validate_bearer_token(config, req, next)
        }));
    }

    // Pre-auth, WebSocket, and metrics routes merged AFTER the auth layer
    // so they bypass bearer middleware:
    // - pre_auth: login/logout/change-expired-password (user has no token yet)
    // - optional signup: only exposed when explicitly enabled and JWT issuance works
    // - ws: browsers can't set Authorization headers on WebSocket connections
    let enable_signup = api_config.allow_public_signup && state.jwt_secret.is_some();
    if api_config.allow_public_signup && !enable_signup {
        tracing::warn!(
            "Public signup requested but auth_token/JWT secret is unavailable; signup route disabled"
        );
    }
    router = router.merge(Router::new().nest("/api/v1", v1::pre_auth_router(enable_signup)));
    router = router.merge(Router::new().nest("/api/v1", v1::ws_router()));

    router
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the API server.
pub async fn serve(
    bind_addr: std::net::SocketAddr,
    state: ApiState,
    api_config: &ApiConfig,
) -> anyhow::Result<()> {
    let app = create_router(state, api_config);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!("API server listening on {}", bind_addr);

    axum::serve(listener, app).await?;

    Ok(())
}
