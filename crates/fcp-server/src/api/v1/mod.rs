//! API v1 routes.

pub mod registrations;
pub mod system;
pub mod users;

use crate::api::state::ApiState;
use axum::{
    routing::{delete, get, put},
    Router,
};

/// Create the v1 API router.
pub fn router() -> Router<ApiState> {
    Router::new()
        // System endpoints
        .route("/health", get(system::health))
        .route("/status", get(system::status))
        // User management endpoints
        .route("/users", get(users::list_users).post(users::create_user))
        .route(
            "/users/:username",
            get(users::get_user)
                .put(users::update_user)
                .delete(users::delete_user),
        )
        .route("/users/:username/password", put(users::update_password))
        // Registration viewing endpoints
        .route(
            "/registrations/:aor",
            get(registrations::get_registrations)
                .delete(registrations::delete_registrations),
        )
        .route(
            "/registrations/:aor/:contact",
            delete(registrations::delete_registration),
        )
}
