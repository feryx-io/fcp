//! API v1 routes.

pub mod debug;
pub mod media;
pub mod registrations;
pub mod system;
pub mod users;

use crate::api::state::ApiState;
use axum::{
    routing::{delete, get, post, put},
    Router,
};

/// Create the v1 API router.
pub fn router() -> Router<ApiState> {
    Router::new()
        // System endpoints
        .route("/health", get(system::health))
        .route("/status", get(system::status))
        // Debug endpoints (for live system inspection)
        .route("/debug/summary", get(debug::debug_summary))
        .route("/debug/dialogs", get(debug::list_dialogs))
        .route("/debug/subscriptions", get(debug::list_subscriptions))
        .route("/debug/calls", get(debug::list_calls))
        .route("/debug/rate-limits", get(debug::list_rate_limits))
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
            get(registrations::get_registrations).delete(registrations::delete_registrations),
        )
        .route(
            "/registrations/:aor/:contact",
            delete(registrations::delete_registration),
        )
        // Media and recording endpoints
        .route("/calls", get(media::list_calls))
        .route(
            "/calls/:call_id/recording/start",
            post(media::start_recording),
        )
        .route(
            "/calls/:call_id/recording/stop",
            post(media::stop_recording),
        )
}
