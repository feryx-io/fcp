//! REST API module.

pub mod error;
pub mod state;
pub mod v1;

use axum::{
    http::{header, Method},
    Router,
};
use state::ApiState;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

/// Create the API router with all routes and middleware.
pub fn create_router(state: ApiState) -> Router {
    // CORS configuration
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE]);

    // Build the API router
    Router::new()
        .nest("/api/v1", v1::router())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the API server.
pub async fn serve(
    bind_addr: std::net::SocketAddr,
    state: ApiState,
) -> anyhow::Result<()> {
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!("API server listening on {}", bind_addr);

    axum::serve(listener, app).await?;

    Ok(())
}
