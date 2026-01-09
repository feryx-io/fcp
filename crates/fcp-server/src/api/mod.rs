//! REST API module.

pub mod auth;
pub mod error;
pub mod state;
pub mod v1;
pub mod validation;

use axum::{
    http::{header, Method},
    middleware, Router,
};
use fcp_core::ApiConfig;
use state::ApiState;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

/// Create the API router with all routes and middleware.
pub fn create_router(state: ApiState, api_config: &ApiConfig) -> Router {
    // CORS configuration
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    // Build the API router
    let mut router = Router::new().nest("/api/v1", v1::router());

    // Conditionally apply authentication middleware
    if api_config.auth_enabled {
        if let Some(auth_token) = &api_config.auth_token {
            tracing::info!("API authentication enabled");
            let auth_config = auth::AuthConfig::new(auth_token.clone());
            router = router.layer(middleware::from_fn(move |req, next| {
                let config = auth_config.clone();
                auth::validate_bearer_token(config, req, next)
            }));
        } else {
            tracing::warn!(
                "API authentication is enabled but no auth_token is configured - authentication disabled"
            );
        }
    }

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
